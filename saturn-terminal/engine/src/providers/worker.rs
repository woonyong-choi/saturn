//! provider 연결 하나를 맡는 작업. engine 루프는 요청을 맡기기만 하고 결과와 이벤트를 메시지로 받는다.
//! 설계: docs/design/providers-and-sessions.md#provider-요청-작업

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use saturn_core::providers::{
    InterruptTarget, ProviderClient, ProviderCommand, ProviderError, SessionHandle, SessionSpec,
};
use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::{AgentId, ChatId, Provider, ProviderSessionId};
use saturn_protocol::input::InputAnswer;
use saturn_protocol::rpc::{ModelInfo, PermissionAnswer};
use tokio::sync::mpsc;

use super::{Adapter, AppliedReader, AppliedSettings, ProviderConnection};
use crate::masked_chain;
use crate::processes::{ProcessGroupId, Supervisor};
use crate::providers::LaunchSpec;
use crate::secrets::Masker;

/// 연결 작업 하나의 번호. 연결을 만들 때마다 새로 붙고 한 번 쓴 번호는 다시 쓰지 않는다. 같은 채팅과 provider의
/// 연결을 교체해도 옛 연결의 늦은 메시지가 새 연결의 것으로 읽히지 않게 가른다. provider가 정하는 session 번호
/// (`ProviderSessionId`)와는 다른 값이고, 그 연결 안에서 열린 session 여럿이 연결 번호 하나를 공유한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ConnectionId(u64);

impl ConnectionId {
    fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

/// 연결 작업이 engine 루프로 보내는 메시지. 한 연결의 메시지는 일어난 순서대로 온다. 모든 메시지는 보낸 연결의
/// 번호를 싣고, engine은 지금 그 키의 연결이거나 그 연결에 맡긴 요청의 결과일 때만 적용한다.
#[derive(Debug)]
pub(crate) enum ProviderMsg {
    Event {
        chat: ChatId,
        provider: Provider,
        connection: ConnectionId,
        event: ProviderEvent,
    },
    /// 연결이 알리는 명령 목록이 바뀌었다. 연결 뒤 처음 알릴 때와 바뀔 때마다 온다.
    Commands {
        chat: ChatId,
        provider: Provider,
        connection: ConnectionId,
        commands: Vec<ProviderCommand>,
    },
    /// 연결의 이벤트 흐름이 끝났다.
    Closed {
        chat: ChatId,
        provider: Provider,
        connection: ConnectionId,
    },
    /// 연결 작업이 패닉하거나 중단돼 끝났다. 맡긴 요청의 결과는 오지 않는다.
    Lost {
        chat: ChatId,
        provider: Provider,
        connection: ConnectionId,
    },
    /// `connection`이 없으면 아직 연결이 없는 요청(연결 맺기)의 결과다.
    Reply {
        chat: ChatId,
        provider: Provider,
        connection: Option<ConnectionId>,
        reply: Reply,
    },
}

/// 루프가 기다리지 않고 맡긴 요청의 결과.
#[derive(Debug)]
pub(crate) enum Reply {
    Connected(Result<Box<Connected>, ProviderError>),
    Opened(Result<SessionHandle, ProviderError>),
    /// 이미 열린 session에 변경분 턴을 보낸 결과.
    HandoffSent(Result<(), ProviderError>),
    Sent(Result<(), ProviderError>),
    Steered(Result<(), ProviderError>),
    /// 입력 전달이 아닌 요청(허가·입력 답, 모델 조회, 맥락 정리의 session 열기)의 결과. `tag`는 맡길 때 받은 번호다.
    Call {
        tag: u64,
        result: CallResult,
    },
}

/// 입력 전달이 아닌 요청이 돌려주는 결과. 요청 종류마다 값이 다르다.
#[derive(Debug)]
pub(crate) enum CallResult {
    Done(Result<(), ProviderError>),
    Opened(Result<SessionHandle, ProviderError>),
    Models(Result<Vec<ModelInfo>, ProviderError>),
    Connected(Result<Box<Connected>, ProviderError>),
}

/// 새로 맺은 연결과, 맺은 직후 받은 모델 목록.
#[derive(Debug)]
pub(crate) struct Connected {
    pub(crate) connection: ProviderConnection,
    pub(crate) models: Result<Vec<ModelInfo>, ProviderError>,
}

/// 요청 결과를 받는 곳.
pub(crate) enum Sink<T> {
    /// 루프에 메시지로 보낸다.
    Notify(fn(T) -> Reply),
    /// 루프에 번호를 붙여 보낸다.
    Call(u64, fn(T) -> CallResult),
    /// 결과를 쓰지 않는다. 실패는 작업이 로그로 남긴다.
    Ignore,
}

/// 연결 작업이 순서대로 하나씩 실행하는 요청. 시작한 요청은 끝까지 한다. 쓰는 도중에 멈추면 줄이 반쯤 쓰이기 때문이다.
enum Op {
    Open {
        spec: SessionSpec,
        attempts: u32,
        sink: Sink<Result<SessionHandle, ProviderError>>,
    },
    Send {
        session: ProviderSessionId,
        text: String,
        attempts: u32,
        sink: Sink<Result<(), ProviderError>>,
    },
    Steer {
        session: ProviderSessionId,
        text: String,
        sink: Sink<Result<(), ProviderError>>,
    },
    /// 깊은 subagent부터 차례로 보내다 연결이 끊겼으면 멈춘다.
    InterruptTree {
        session: ProviderSessionId,
        targets: Vec<InterruptTarget>,
    },
    AnswerPermission {
        session: ProviderSessionId,
        request_id: String,
        answer: PermissionAnswer,
        sink: Sink<Result<(), ProviderError>>,
    },
    AnswerInput {
        session: ProviderSessionId,
        request_id: String,
        answer: InputAnswer,
        sink: Sink<Result<(), ProviderError>>,
    },
    Close {
        session: ProviderSessionId,
        sink: Sink<Result<(), ProviderError>>,
    },
    ListModels {
        sink: Sink<Result<Vec<ModelInfo>, ProviderError>>,
    },
    StartQueuedTurn {
        agent: AgentId,
    },
}

/// 루프가 동기로 묻는 연결 정보. 작업이 session을 열 때 채운다.
#[derive(Default)]
struct Shared {
    /// 모든 session이 같이 쓰는 프로세스 묶음.
    shared_group: Option<ProcessGroupId>,
    sessions: Mutex<HashMap<ProviderSessionId, SessionView>>,
}

struct SessionView {
    group: Option<ProcessGroupId>,
    applied: Option<AppliedReader>,
}

impl std::fmt::Debug for Shared {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Shared")
            .field("shared_group", &self.shared_group)
            .finish_non_exhaustive()
    }
}

impl Shared {
    fn new(connection: &ProviderConnection) -> Self {
        Self {
            shared_group: connection.shared_group(),
            ..Self::default()
        }
    }

    fn sessions(&self) -> MutexGuard<'_, HashMap<ProviderSessionId, SessionView>> {
        // 잠금을 쥔 채 panic하는 코드가 없으니 독이 든 잠금도 내용은 온전하다
        self.sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// 작업이 쓰는 값.
struct Context {
    chat: ChatId,
    provider: Provider,
    connection: ConnectionId,
    msgs: mpsc::UnboundedSender<ProviderMsg>,
    masker: Masker,
    shared: Arc<Shared>,
}

impl Context {
    fn put<T>(&self, sink: Sink<T>, value: T) {
        match sink {
            Sink::Notify(wrap) => {
                let _ = self.msgs.send(ProviderMsg::Reply {
                    chat: self.chat,
                    provider: self.provider,
                    connection: Some(self.connection),
                    reply: wrap(value),
                }); // engine이 끝난 뒤에는 받을 곳이 없다
            }
            Sink::Call(tag, wrap) => {
                let _ = self.msgs.send(ProviderMsg::Reply {
                    chat: self.chat,
                    provider: self.provider,
                    connection: Some(self.connection),
                    reply: Reply::Call {
                        tag,
                        result: wrap(value),
                    },
                }); // engine이 끝난 뒤에는 받을 곳이 없다
            }
            Sink::Ignore => {}
        }
    }

    /// 명령 목록이 마지막으로 알린 것과 다르면 알린다. 목록이 처음부터 비어 있으면 알리지 않는다. engine이 끝났으면
    /// 거짓.
    fn announce_commands(
        &self,
        connection: &ProviderConnection,
        announced: &mut Vec<ProviderCommand>,
    ) -> bool {
        let current = connection.commands();
        if current == *announced {
            return true;
        }
        announced.clone_from(&current);
        self.msgs
            .send(ProviderMsg::Commands {
                chat: self.chat,
                provider: self.provider,
                connection: self.connection,
                commands: current,
            })
            .is_ok()
    }

    fn warn(&self, what: &str, error: &ProviderError) {
        tracing::warn!(error = %masked_chain(&self.masker, error), "{what}");
    }

    fn register(&self, connection: &ProviderConnection, session: &ProviderSessionId) {
        self.shared.sessions().insert(
            session.clone(),
            SessionView {
                group: connection.process_group(session),
                applied: connection.applied_reader(session),
            },
        );
    }

    async fn execute(&self, connection: &mut ProviderConnection, op: Op) {
        match op {
            Op::Open {
                spec,
                attempts,
                sink,
            } => {
                let opened = open_with_retries(connection, spec, attempts).await;
                if let Ok(handle) = &opened {
                    self.register(connection, &handle.provider_session);
                }
                self.put(sink, opened);
            }
            Op::Send {
                session,
                text,
                attempts,
                sink,
            } => {
                let sent = send_with_retries(connection, &session, &text, attempts).await;
                self.put(sink, sent);
            }
            Op::Steer {
                session,
                text,
                sink,
            } => {
                let steered = connection.steer(&session, &text).await;
                self.put(sink, steered);
            }
            Op::InterruptTree { session, targets } => {
                for target in targets {
                    match connection.interrupt(&session, target).await {
                        Ok(()) => {}
                        Err(ProviderError::ConnectionLost) => break,
                        Err(error) => self.warn("interrupt was not delivered", &error),
                    }
                }
            }
            Op::AnswerPermission {
                session,
                request_id,
                answer,
                sink,
            } => {
                let answered = connection
                    .answer_permission(&session, &request_id, answer)
                    .await;
                self.put(sink, answered);
            }
            Op::AnswerInput {
                session,
                request_id,
                answer,
                sink,
            } => {
                let answered = connection.answer_input(&session, &request_id, answer).await;
                self.put(sink, answered);
            }
            Op::Close { session, sink } => {
                let closed = connection.close_session(&session).await;
                self.shared.sessions().remove(&session);
                if let (Err(error), Sink::Ignore) = (&closed, &sink) {
                    self.warn("failed to close session", error);
                }
                self.put(sink, closed);
            }
            Op::ListModels { sink } => {
                let models = connection.list_models().await;
                self.put(sink, models);
            }
            Op::StartQueuedTurn { agent } => connection.start_queued_turn(agent).await,
        }
    }
}

/// 열지 못하는 `NotSent`(재개 실패)만 다시 연다.
async fn open_with_retries(
    connection: &mut ProviderConnection,
    spec: SessionSpec,
    attempts: u32,
) -> Result<SessionHandle, ProviderError> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        let opened = connection.open_session(spec.clone()).await;
        match opened {
            Err(ProviderError::NotSent { .. }) if attempt < attempts => {
                tracing::warn!(attempt, "session was not opened, trying again");
            }
            other => return other,
        }
    }
}

/// 보내기 전에 확정된 실패만 다시 보낸다.
async fn send_with_retries(
    connection: &mut ProviderConnection,
    session: &ProviderSessionId,
    text: &str,
    attempts: u32,
) -> Result<(), ProviderError> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        let sent = connection.send_turn(session, text).await;
        match sent {
            Err(ProviderError::NotSent { .. }) if attempt < attempts => {
                tracing::warn!(attempt, "turn was not sent, sending again");
            }
            other => return other,
        }
    }
}

/// 요청을 연결 작업에 맡기는 쪽. 버리면 맡긴 요청을 마친 뒤 연결 작업이 끝나고 연결을 닫는다.
#[derive(Debug)]
pub(crate) struct ProviderHandle {
    chat: ChatId,
    provider: Provider,
    id: ConnectionId,
    ops: mpsc::UnboundedSender<Op>,
    msgs: mpsc::UnboundedSender<ProviderMsg>,
    shared: Arc<Shared>,
}

impl ProviderHandle {
    /// `connection`을 맡는 작업을 띄운다. 이벤트와 결과는 `msgs`로 간다.
    pub(crate) fn spawn(
        connection: ProviderConnection,
        chat: ChatId,
        msgs: mpsc::UnboundedSender<ProviderMsg>,
        masker: Masker,
    ) -> Self {
        let provider = connection.provider();
        let id = ConnectionId::next();
        let shared = Arc::new(Shared::new(&connection));
        let (ops, requests) = mpsc::unbounded_channel();
        let context = Context {
            chat,
            provider,
            connection: id,
            msgs: msgs.clone(),
            masker,
            shared: Arc::clone(&shared),
        };
        let task = tokio::spawn(run(connection, requests, context));
        watch(task, (chat, provider), msgs.clone(), Err(id));
        Self {
            chat,
            provider,
            id,
            ops,
            msgs,
            shared,
        }
    }

    /// 이 연결의 번호.
    pub(crate) fn id(&self) -> ConnectionId {
        self.id
    }

    /// 모르는 session이면 `None`.
    pub(crate) fn process_group(&self, session: &ProviderSessionId) -> Option<ProcessGroupId> {
        self.shared
            .shared_group
            .or_else(|| self.shared.sessions().get(session)?.group)
    }

    /// 모든 session이 프로세스 묶음 하나를 같이 쓰는 provider의 그 묶음.
    pub(crate) fn shared_group(&self) -> Option<ProcessGroupId> {
        self.shared.shared_group
    }

    /// 적용값을 받기 전이면 `None`.
    pub(crate) fn applied_settings(&self, session: &ProviderSessionId) -> Option<AppliedSettings> {
        let reader = self.shared.sessions().get(session)?.applied.clone()?;
        reader()
    }

    /// 요청을 줄 세운다. 연결 작업이 이미 끝났으면 `lost`를 결과로 보낸다.
    fn submit(&self, op: Op, lost: Option<Reply>) {
        if self.ops.send(op).is_err()
            && let Some(reply) = lost
        {
            let _ = self.msgs.send(ProviderMsg::Reply {
                chat: self.chat,
                provider: self.provider,
                connection: Some(self.id),
                reply,
            }); // engine이 끝난 뒤에는 받을 곳이 없다
        }
    }

    /// 열기 요청을 맡기고 기다리지 않는다. 결과는 `Reply::Opened`로 온다.
    pub(crate) fn open_session_detached(&self, spec: SessionSpec, attempts: u32) {
        self.submit(
            Op::Open {
                spec,
                attempts,
                sink: Sink::Notify(Reply::Opened),
            },
            Some(Reply::Opened(Err(ProviderError::ConnectionLost))),
        );
    }

    /// 결과는 `Reply::Sent`로 온다.
    pub(crate) fn send_turn_detached(
        &self,
        session: ProviderSessionId,
        text: String,
        attempts: u32,
    ) {
        self.submit(
            Op::Send {
                session,
                text,
                attempts,
                sink: Sink::Notify(Reply::Sent),
            },
            Some(Reply::Sent(Err(ProviderError::ConnectionLost))),
        );
    }

    /// 열린 session에 변경분 턴을 맡긴다. 결과는 `Reply::HandoffSent`로 온다.
    pub(crate) fn send_handoff_detached(&self, session: ProviderSessionId, text: String) {
        self.submit(
            Op::Send {
                session,
                text,
                attempts: 1,
                sink: Sink::Notify(Reply::HandoffSent),
            },
            Some(Reply::HandoffSent(Err(ProviderError::ConnectionLost))),
        );
    }

    /// 결과는 `Reply::Steered`로 온다.
    pub(crate) fn steer_detached(&self, session: ProviderSessionId, text: String) {
        self.submit(
            Op::Steer {
                session,
                text,
                sink: Sink::Notify(Reply::Steered),
            },
            Some(Reply::Steered(Err(ProviderError::ConnectionLost))),
        );
    }

    /// 멈춤 신호를 기다리지 않고 맡긴다. 앞선 요청 뒤에 실행된다.
    pub(crate) fn interrupt_tree_detached(
        &self,
        session: ProviderSessionId,
        targets: Vec<InterruptTarget>,
    ) {
        self.submit(Op::InterruptTree { session, targets }, None);
    }

    /// 턴 완료 뒤 줄 세워 둔 첫 입력을 보내도록 맡긴다. 기다리지 않는다.
    pub(crate) fn start_queued_turn_detached(&self, agent: AgentId) {
        self.submit(Op::StartQueuedTurn { agent }, None);
    }

    /// 결과를 쓰지 않고 session을 닫도록 맡긴다. 실패는 연결 작업이 로그로 남긴다.
    pub(crate) fn close_session_detached(&self, session: ProviderSessionId) {
        self.submit(
            Op::Close {
                session,
                sink: Sink::Ignore,
            },
            None,
        );
    }

    /// 열기 요청을 맡기고 기다리지 않는다. 결과는 `CallResult::Opened`로 온다. 맥락 정리가 쓴다.
    pub(crate) fn open_session_call(&self, tag: u64, spec: SessionSpec, attempts: u32) {
        self.submit(
            Op::Open {
                spec,
                attempts,
                sink: Sink::Call(tag, CallResult::Opened),
            },
            Some(Reply::Call {
                tag,
                result: CallResult::Opened(Err(ProviderError::ConnectionLost)),
            }),
        );
    }

    /// 허가 답을 맡기고 기다리지 않는다. 결과는 `CallResult::Done`으로 온다.
    pub(crate) fn answer_permission_call(
        &self,
        tag: u64,
        session: ProviderSessionId,
        request_id: String,
        answer: PermissionAnswer,
    ) {
        self.submit(
            Op::AnswerPermission {
                session,
                request_id,
                answer,
                sink: Sink::Call(tag, CallResult::Done),
            },
            Some(Reply::Call {
                tag,
                result: CallResult::Done(Err(ProviderError::ConnectionLost)),
            }),
        );
    }

    /// 입력 요청 답을 맡기고 기다리지 않는다. 결과는 `CallResult::Done`으로 온다.
    pub(crate) fn answer_input_call(
        &self,
        tag: u64,
        session: ProviderSessionId,
        request_id: String,
        answer: InputAnswer,
    ) {
        self.submit(
            Op::AnswerInput {
                session,
                request_id,
                answer,
                sink: Sink::Call(tag, CallResult::Done),
            },
            Some(Reply::Call {
                tag,
                result: CallResult::Done(Err(ProviderError::ConnectionLost)),
            }),
        );
    }

    /// 모델 목록 조회를 맡기고 기다리지 않는다. 결과는 `CallResult::Models`로 온다.
    pub(crate) fn list_models_call(&self, tag: u64) {
        self.submit(
            Op::ListModels {
                sink: Sink::Call(tag, CallResult::Models),
            },
            Some(Reply::Call {
                tag,
                result: CallResult::Models(Err(ProviderError::ConnectionLost)),
            }),
        );
    }
}

/// 요청과 이벤트를 번갈아 처리한다. 요청이 먼저다. 요청을 실행하는 동안 온 이벤트는 그 결과 뒤에 전달된다.
async fn run(
    mut connection: ProviderConnection,
    mut requests: mpsc::UnboundedReceiver<Op>,
    context: Context,
) {
    let mut is_streaming = true;
    let mut announced: Vec<ProviderCommand> = Vec::new();
    loop {
        if !context.announce_commands(&connection, &mut announced) {
            break; // engine이 끝났다
        }
        tokio::select! {
            biased;
            request = requests.recv() => {
                let Some(op) = request else { break };
                context.execute(&mut connection, op).await;
            }
            event = connection.next_event(), if is_streaming => {
                let message = match event {
                    Some(event) => ProviderMsg::Event {
                        chat: context.chat,
                        provider: context.provider,
                        connection: context.connection,
                        event,
                    },
                    None => {
                        is_streaming = false;
                        ProviderMsg::Closed {
                            chat: context.chat,
                            provider: context.provider,
                            connection: context.connection,
                        }
                    }
                };
                if context.msgs.send(message).is_err() {
                    break; // engine이 끝났다
                }
            }
        }
    }
}

/// 연결을 맺고 모델 목록까지 받아 보낸다. 루프는 기다리지 않는다. `tag`가 없으면 입력 전달이 기다리는
/// `Reply::Connected`로, 있으면 그 번호의 `CallResult::Connected`로 보낸다.
pub(crate) fn spawn_connect(
    chat: ChatId,
    launch: LaunchSpec,
    adapter: Arc<dyn Adapter>,
    supervisor: Supervisor,
    msgs: mpsc::UnboundedSender<ProviderMsg>,
    tag: Option<u64>,
) {
    let provider = launch.provider;
    let lost = msgs.clone();
    let reply = move |connected: Result<Box<Connected>, ProviderError>| match tag {
        Some(tag) => Reply::Call {
            tag,
            result: CallResult::Connected(connected),
        },
        None => Reply::Connected(connected),
    };
    let task = tokio::spawn(async move {
        let connected = match adapter.connect(launch, supervisor).await {
            Ok(mut connection) => {
                let models = connection.list_models().await;
                Ok(Box::new(Connected { connection, models }))
            }
            Err(error) => Err(error),
        };
        let _ = msgs.send(ProviderMsg::Reply {
            chat,
            provider,
            connection: None,
            reply: reply(connected),
        }); // engine이 끝난 뒤에는 받을 곳이 없다
    });
    let lost_reply = match tag {
        Some(tag) => Reply::Call {
            tag,
            result: CallResult::Connected(Err(ProviderError::ConnectionLost)),
        },
        None => Reply::Connected(Err(ProviderError::ConnectionLost)),
    };
    watch(task, (chat, provider), lost, Ok(lost_reply));
}

/// 작업이 패닉하거나 중단돼 끝나면 루프에 알린다. 정상으로 끝난 작업(핸들을 버려 닫은 연결)은 알리지 않는다.
/// 연결 맺기 작업(`Ok(reply)`)은 그 결과를 기다리던 전달에 `reply`를 대신 돌려주고, 연결 작업(`Err(id)`)은
/// 그 연결의 `Lost`를 알린다.
fn watch(
    task: tokio::task::JoinHandle<()>,
    (chat, provider): (ChatId, Provider),
    msgs: mpsc::UnboundedSender<ProviderMsg>,
    outcome: Result<Reply, ConnectionId>,
) {
    tokio::spawn(async move {
        let Err(error) = task.await else { return };
        tracing::warn!(%error, "provider task ended unexpectedly");
        let message = match outcome {
            Ok(reply) => ProviderMsg::Reply {
                chat,
                provider,
                connection: None,
                reply,
            },
            Err(connection) => ProviderMsg::Lost {
                chat,
                provider,
                connection,
            },
        };
        let _ = msgs.send(message); // engine이 끝난 뒤에는 받을 곳이 없다
    });
}
