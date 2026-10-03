//! provider 연결 하나를 맡는 작업. engine 루프는 요청을 맡기기만 하고 결과와 이벤트를 메시지로 받는다.
//! 설계: docs/design/providers-and-sessions.md#provider-요청-작업

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use saturn_core::providers::{
    InterruptTarget, ProviderClient, ProviderError, SessionHandle, SessionSpec,
};
use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::{AgentId, ChatId, Provider, ProviderSessionId};
use saturn_protocol::input::InputAnswer;
use saturn_protocol::rpc::{ModelInfo, PermissionAnswer};
use tokio::sync::{mpsc, oneshot};

use super::{AppliedSettings, ProviderConnection};
use crate::masked_chain;
use crate::processes::{ProcessGroupId, Supervisor};
use crate::providers::LaunchSpec;
use crate::secrets::Masker;

/// 열린 session의 적용값을 읽는 함수. 읽기 작업이 갱신하는 값이라 부를 때마다 읽는다.
pub(crate) type AppliedReader = Arc<dyn Fn() -> Option<AppliedSettings> + Send + Sync>;

/// 연결 작업이 engine 루프로 보내는 메시지. 한 연결의 메시지는 일어난 순서대로 온다.
#[derive(Debug)]
pub(crate) enum ProviderMsg {
    Event {
        chat: ChatId,
        provider: Provider,
        event: ProviderEvent,
    },
    /// 연결의 이벤트 흐름이 끝났다.
    Closed { chat: ChatId, provider: Provider },
    /// 연결 작업이 패닉하거나 중단돼 끝났다. 맡긴 요청의 결과는 오지 않는다.
    Lost { chat: ChatId, provider: Provider },
    Reply {
        chat: ChatId,
        provider: Provider,
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
}

/// 새로 맺은 연결과, 맺은 직후 받은 모델 목록.
#[derive(Debug)]
pub(crate) struct Connected {
    pub(crate) connection: ProviderConnection,
    pub(crate) models: Result<Vec<ModelInfo>, ProviderError>,
}

/// 요청 결과를 받는 곳.
pub(crate) enum Sink<T> {
    /// 호출자가 기다린다.
    Wait(oneshot::Sender<T>),
    /// 루프에 메시지로 보낸다.
    Notify(fn(T) -> Reply),
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
    #[cfg(test)]
    fake: Option<super::test_support::FakeProvider>,
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
        #[cfg(test)]
        if let ProviderConnection::Fake(fake) = connection {
            return Self {
                fake: Some(fake.clone()),
                ..Self::default()
            };
        }
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
    msgs: mpsc::UnboundedSender<ProviderMsg>,
    masker: Masker,
    shared: Arc<Shared>,
}

impl Context {
    fn put<T>(&self, sink: Sink<T>, value: T) {
        match sink {
            Sink::Wait(reply) => {
                let _ = reply.send(value); // 기다리던 호출자가 이미 사라졌다
            }
            Sink::Notify(wrap) => {
                let _ = self.msgs.send(ProviderMsg::Reply {
                    chat: self.chat,
                    provider: self.provider,
                    reply: wrap(value),
                }); // engine이 끝난 뒤에는 받을 곳이 없다
            }
            Sink::Ignore => {}
        }
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
        let shared = Arc::new(Shared::new(&connection));
        let (ops, requests) = mpsc::unbounded_channel();
        let context = Context {
            chat,
            provider,
            msgs: msgs.clone(),
            masker,
            shared: Arc::clone(&shared),
        };
        let task = tokio::spawn(run(connection, requests, context));
        watch(task, chat, provider, msgs.clone(), None);
        Self {
            chat,
            provider,
            ops,
            msgs,
            shared,
        }
    }

    /// 모르는 session이면 `None`.
    pub(crate) fn process_group(&self, session: &ProviderSessionId) -> Option<ProcessGroupId> {
        #[cfg(test)]
        if let Some(fake) = &self.shared.fake {
            return fake.group();
        }
        self.shared
            .shared_group
            .or_else(|| self.shared.sessions().get(session)?.group)
    }

    /// 모든 session이 프로세스 묶음 하나를 같이 쓰는 provider의 그 묶음.
    pub(crate) fn shared_group(&self) -> Option<ProcessGroupId> {
        #[cfg(test)]
        if let Some(fake) = &self.shared.fake {
            return fake.group();
        }
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
                reply,
            }); // engine이 끝난 뒤에는 받을 곳이 없다
        }
    }

    async fn wait<T>(
        &self,
        make: impl FnOnce(Sink<Result<T, ProviderError>>) -> Op,
    ) -> Result<T, ProviderError> {
        let (reply, receive) = oneshot::channel();
        if self.ops.send(make(Sink::Wait(reply))).is_err() {
            return Err(ProviderError::ConnectionLost);
        }
        receive.await.unwrap_or(Err(ProviderError::ConnectionLost))
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

    /// 열기 요청을 맡기고 끝날 때까지 기다린다.
    pub(crate) async fn open_session(
        &self,
        spec: SessionSpec,
        attempts: u32,
    ) -> Result<SessionHandle, ProviderError> {
        self.wait(|sink| Op::Open {
            spec,
            attempts,
            sink,
        })
        .await
    }

    pub(crate) async fn answer_permission(
        &self,
        session: &ProviderSessionId,
        request_id: &str,
        answer: PermissionAnswer,
    ) -> Result<(), ProviderError> {
        self.wait(|sink| Op::AnswerPermission {
            session: session.clone(),
            request_id: request_id.to_owned(),
            answer,
            sink,
        })
        .await
    }

    pub(crate) async fn answer_input(
        &self,
        session: &ProviderSessionId,
        request_id: &str,
        answer: InputAnswer,
    ) -> Result<(), ProviderError> {
        self.wait(|sink| Op::AnswerInput {
            session: session.clone(),
            request_id: request_id.to_owned(),
            answer,
            sink,
        })
        .await
    }

    pub(crate) async fn close_session(
        &self,
        session: &ProviderSessionId,
    ) -> Result<(), ProviderError> {
        self.wait(|sink| Op::Close {
            session: session.clone(),
            sink,
        })
        .await
    }

    pub(crate) async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        self.wait(|sink| Op::ListModels { sink }).await
    }
}

/// 요청과 이벤트를 번갈아 처리한다. 요청이 먼저다. 요청을 실행하는 동안 온 이벤트는 그 결과 뒤에 전달된다.
async fn run(
    mut connection: ProviderConnection,
    mut requests: mpsc::UnboundedReceiver<Op>,
    context: Context,
) {
    let mut is_streaming = true;
    loop {
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
                        event,
                    },
                    None => {
                        is_streaming = false;
                        ProviderMsg::Closed {
                            chat: context.chat,
                            provider: context.provider,
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

/// 연결을 맺고 모델 목록까지 받아 `Reply::Connected`로 보낸다. 루프는 기다리지 않는다.
pub(crate) fn spawn_connect(
    chat: ChatId,
    launch: LaunchSpec,
    supervisor: Supervisor,
    msgs: mpsc::UnboundedSender<ProviderMsg>,
) {
    let provider = launch.provider;
    let lost = msgs.clone();
    let task = tokio::spawn(async move {
        let connected = match ProviderConnection::connect(launch, supervisor).await {
            Ok(mut connection) => {
                let models = connection.list_models().await;
                Ok(Box::new(Connected { connection, models }))
            }
            Err(error) => Err(error),
        };
        let _ = msgs.send(ProviderMsg::Reply {
            chat,
            provider,
            reply: Reply::Connected(connected),
        }); // engine이 끝난 뒤에는 받을 곳이 없다
    });
    watch(
        task,
        chat,
        provider,
        lost,
        Some(Reply::Connected(Err(ProviderError::ConnectionLost))),
    );
}

/// 작업이 패닉하거나 중단돼 끝나면 루프에 알린다. 정상으로 끝난 작업(핸들을 버려 닫은 연결)은 알리지 않는다.
/// `reply`가 있으면 그 결과를 기다리던 전달에 대신 돌려준다.
fn watch(
    task: tokio::task::JoinHandle<()>,
    chat: ChatId,
    provider: Provider,
    msgs: mpsc::UnboundedSender<ProviderMsg>,
    reply: Option<Reply>,
) {
    tokio::spawn(async move {
        let Err(error) = task.await else { return };
        tracing::warn!(%error, "provider task ended unexpectedly");
        let message = match reply {
            Some(reply) => ProviderMsg::Reply {
                chat,
                provider,
                reply,
            },
            None => ProviderMsg::Lost { chat, provider },
        };
        let _ = msgs.send(message); // engine이 끝난 뒤에는 받을 곳이 없다
    });
}
