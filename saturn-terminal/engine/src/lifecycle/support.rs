//! 입력 흐름 테스트 공용 도구: 소켓 없이 붙인 채팅, 가짜 provider, 가짜 router 답.

use saturn_core::queue::{Permission, QueuedInput};
use saturn_core::routers::{RELATION_OPTIONS, RouteDecision, SEND_OPTIONS};
use saturn_protocol::event::{
    Activity, PermissionCall, PermissionTool, ProviderEvent, ToolCategory, ToolDetail, TurnOrigin,
};
use saturn_protocol::ids::{AgentId, ChatId, InputId, Provider, SubagentId};
use saturn_protocol::input::{InputAnswer, InputField, InputFieldKind, InputRequest};
use saturn_protocol::rpc::{ModelChoice, PermissionAnswer};
use saturn_protocol::state::InputState;
use serde_json::{Value, json};

use super::*;
use crate::Attachment;
use crate::chat_env::ChatEnv;
use crate::flow::RouterJob;
use crate::providers::ProviderConnection;
use crate::providers::test_support::FakeProvider;
use crate::rpc::ClientId;
use crate::store::NewInput;

pub(super) const CLIENT: ClientId = ClientId(1);

/// 두 번째 채팅에 붙은 TUI.
pub(super) const OTHER_CLIENT: ClientId = ClientId(2);

/// `Flow`에 더한 두 번째 채팅과 그 채팅의 가짜 Claude, 열린 에이전트.
pub(super) struct OtherChat {
    pub(super) chat: ChatId,
    pub(super) fake: FakeProvider,
    pub(super) agent: AgentId,
}

/// 소켓 없이 붙인 채팅 하나와, 그 채팅에 연결된 가짜 Claude를 가진 engine.
pub(super) struct Flow {
    pub(super) engine: Engine,
    pub(super) fake: FakeProvider,
    pub(super) transport: Arc<FakeTransport>,
    pub(super) chat: ChatId,
    pub(super) fixture: Fixture,
}

impl Flow {
    /// `router_replies`는 시작 확인 뒤 router가 차례로 낼 답이다.
    pub(super) async fn new(router_replies: Vec<FakeReply>) -> Self {
        Self::with_config("", router_replies).await
    }

    /// `config`는 시작 전에 쓰는 사용자 설정 파일이다.
    pub(super) async fn with_config(config: &str, router_replies: Vec<FakeReply>) -> Self {
        Self::with_setup(config, &[], router_replies).await
    }

    /// `overrides`는 실행 `-c` 층의 `키=값`이다.
    pub(super) async fn with_setup(
        config: &str,
        overrides: &[&str],
        router_replies: Vec<FakeReply>,
    ) -> Self {
        let mut fixture = Fixture::new();
        fixture.options.run_overrides = overrides.iter().map(|item| (*item).to_owned()).collect();
        if !config.is_empty() {
            fixture.write_user_config(config);
        }
        let mut script = check_passes();
        script.extend(router_replies);
        let transport = FakeTransport::new(script);
        let env = fixture.env(true, Arc::clone(&transport)).await;
        let mut engine = fixture.start(env).await.unwrap();
        let chat = engine
            .store
            .create_chat(fixture.workdir.clone())
            .await
            .unwrap();
        engine.chats.insert(
            chat,
            ChatEnv::new(
                fixture.workdir.clone(),
                vec![("PATH".to_owned(), "/nonexistent".to_owned())],
            ),
        );
        engine.attachments.insert(
            CLIENT,
            Attachment {
                chat,
                overrides: Vec::new(),
                folder_trust: None,
            },
        );
        let fake = FakeProvider::new(Provider::Claude);
        let questions = match engine.settings.current() {
            Some(revision) => engine
                .agent_questions(chat, revision)
                .await
                .expect("agent questions should be readable"),
            None => true,
        };
        engine
            .flow
            .questions_of_connection
            .insert((chat, Provider::Claude), questions);
        engine.providers.insert(
            (chat, Provider::Claude),
            ProviderConnection::Fake(fake.clone()),
        );
        Self {
            engine,
            fake,
            transport,
            chat,
            fixture,
        }
    }

    /// 다른 작업 폴더의 두 번째 채팅을 `OTHER_CLIENT`로 붙이고 입력 하나를 보내 에이전트를 연다. router 답이 하나 더 필요하다.
    /// 같은 폴더면 쓰기 잠금 때문에 입력이 기다린다.
    pub(super) async fn open_other_chat(&mut self) -> OtherChat {
        let workdir = self.fixture.workdir.with_file_name("other-work");
        std::fs::create_dir_all(&workdir).unwrap();
        let chat = self
            .engine
            .store
            .create_chat(workdir.clone())
            .await
            .unwrap();
        self.engine.chats.insert(
            chat,
            ChatEnv::new(
                workdir,
                vec![("PATH".to_owned(), "/nonexistent".to_owned())],
            ),
        );
        self.engine.attachments.insert(
            OTHER_CLIENT,
            Attachment {
                chat,
                overrides: Vec::new(),
                folder_trust: None,
            },
        );
        let fake = FakeProvider::new(Provider::Claude);
        self.engine
            .flow
            .questions_of_connection
            .insert((chat, Provider::Claude), true);
        self.engine.providers.insert(
            (chat, Provider::Claude),
            ProviderConnection::Fake(fake.clone()),
        );
        self.engine
            .submit_input(OTHER_CLIENT, chat, 1, "other work".to_owned(), false)
            .await
            .unwrap();
        self.settle().await;
        let agent = self
            .engine
            .flow
            .live
            .values()
            .find(|live| {
                self.engine
                    .session_chat(live.session)
                    .is_ok_and(|owner| owner == chat)
            })
            .map(|live| live.agent)
            .expect("a session should be open in the other chat");
        OtherChat { chat, fake, agent }
    }

    /// 열려 있는 가짜 Claude가 낸 것처럼 이벤트를 처리한다.
    pub(super) async fn claude_event(&mut self, event: ProviderEvent) {
        self.event(Provider::Claude, event).await;
    }

    pub(super) async fn event(&mut self, provider: Provider, event: ProviderEvent) {
        self.engine
            .on_provider_event(provider, event)
            .await
            .expect("event should be handled");
    }

    /// 채팅에 다른 provider의 가짜 연결을 더한다.
    pub(super) fn add_provider(&mut self, provider: Provider) -> FakeProvider {
        let fake = FakeProvider::new(provider);
        self.engine
            .flow
            .questions_of_connection
            .insert((self.chat, provider), true);
        self.engine.providers.insert(
            (self.chat, provider),
            ProviderConnection::Fake(fake.clone()),
        );
        fake
    }

    /// 소켓으로 붙은 TUI. 붙는 동안 engine 요청 처리를 돌리고, 그 뒤의 알림은 직접 부른 처리에서 온다.
    pub(super) async fn client(&mut self) -> Client {
        self.attach().await.0
    }

    /// `client`와 같고 붙을 때 받은 알림도 돌려준다.
    pub(super) async fn attach(&mut self) -> (Client, Vec<Notification>) {
        let mut client = Client::connect(&self.fixture.socket()).await;
        let chat = self.chat;
        let workdir = self.fixture.workdir.display().to_string();
        let greeting = drive(&mut self.engine, async {
            client
                .attach(
                    1,
                    Request::Attach {
                        chat: Some(chat),
                        workdir,
                        env: vec![("PATH".to_owned(), "/nonexistent".to_owned())],
                        overrides: Vec::new(),
                        add_dirs: Vec::new(),
                    },
                )
                .await
        })
        .await;
        (client, greeting)
    }

    /// 가짜 provider가 낸 요청 ID에 engine이 발급한 허가 요청 ID. 아직 올라오지 않았으면 `None`.
    pub(super) fn permission_id(&self, provider_request: &str) -> Option<String> {
        self.engine
            .flow
            .permissions
            .iter()
            .find(|(_, pending)| pending.provider_request == provider_request)
            .map(|(id, _)| id.clone())
    }

    /// `permission_id`의 입력 요청 쪽.
    pub(super) fn input_id(&self, provider_request: &str) -> Option<String> {
        self.engine
            .flow
            .inputs
            .iter()
            .find(|(_, pending)| pending.provider_request == provider_request)
            .map(|(id, _)| id.clone())
    }

    /// 채팅에 붙은 TUI(`CLIENT`)가 provider 요청 ID로 올라온 허가 요청에 답한다. 올라오지 않은 ID는 그대로 넘긴다.
    pub(super) async fn answer_permission(
        &mut self,
        provider_request: &str,
        answer: PermissionAnswer,
    ) -> Result<(), EngineError> {
        let id = self
            .permission_id(provider_request)
            .unwrap_or_else(|| provider_request.to_owned());
        self.engine.answer_permission(CLIENT, id, answer).await
    }

    /// `answer_permission`의 입력 요청 쪽.
    pub(super) async fn answer_input(
        &mut self,
        provider_request: &str,
        answer: InputAnswer,
    ) -> Result<(), EngineError> {
        let id = self
            .input_id(provider_request)
            .unwrap_or_else(|| provider_request.to_owned());
        self.engine.answer_input(CLIENT, id, answer).await
    }

    /// router 호출 수. 시작 확인의 두 호출은 뺀다.
    pub(super) fn router_calls(&self) -> usize {
        self.transport.calls().len() - check_passes().len()
    }

    /// 채팅의 고정 모델을 `/model`처럼 저장한다.
    pub(super) async fn pin(&mut self, model: &ModelChoice) {
        self.engine
            .set_model(CLIENT, self.chat, model)
            .await
            .expect("model should be pinned");
    }

    pub(super) async fn submit(&mut self, text: &str) -> InputId {
        self.submit_with(text, None, false).await
    }

    pub(super) async fn submit_with(
        &mut self,
        text: &str,
        pinned_model: Option<ModelChoice>,
        skip_relation: bool,
    ) -> InputId {
        if let Some(model) = &pinned_model {
            self.pin(model).await;
        }
        let before = self.latest_input().await;
        self.engine
            .submit_input(CLIENT, self.chat, 1, text.to_owned(), skip_relation)
            .await
            .unwrap();
        self.settle().await;
        let after = self.latest_input().await;
        assert_ne!(before, after, "input should have been accepted");
        after.expect("input should exist")
    }

    /// 끝난 입력도 기록에 남아 있으므로 기록의 마지막 입력 번호를 본다.
    async fn latest_input(&self) -> Option<InputId> {
        let (entries, _) = self
            .engine
            .store
            .recent_history(self.chat, 500)
            .await
            .unwrap();
        entries
            .into_iter()
            .filter_map(|entry| match entry {
                crate::store::HistoryEntry::Input { input, .. } => Some(input),
                crate::store::HistoryEntry::Event { .. } => None,
            })
            .max()
    }

    /// 판단과 전송 없이 접수만 한다. 판단 적용 시험이 판단 차례를 직접 다루는 데 쓴다.
    pub(super) async fn accept_only(&mut self, text: &str) -> InputId {
        let new = NewInput {
            chat: self.chat,
            text: text.to_owned(),
            settings: self
                .engine
                .settings
                .current()
                .expect("settings should be applied at start"),
            permission: Permission::Write,
            workdir: self.fixture.workdir.clone(),
            pinned_model: None,
            skip_relation: false,
        };
        let id = self.engine.store.accept_input(&new).await.unwrap();
        self.engine.queue.accept(QueuedInput {
            id,
            chat: new.chat,
            text: new.text,
            settings: new.settings,
            permission: new.permission,
            workdir: new.workdir,
            pinned_model: None,
            skip_relation: false,
            state: InputState::Judging,
            reason: None,
            task: None,
        });
        id
    }

    /// 지금 revision으로 router에 한 번 묻고 그 판단을 돌려준다. 기록은 적용 때 쓴다.
    pub(super) async fn router_now(&mut self, input: InputId, running: bool) -> RouteDecision {
        let record = self.record(input);
        let revision = self.engine.queue.revision(self.chat);
        let request = self.engine.router_request(&record, running);
        let exchange = self.engine.routers.shared().exchange(request.clone()).await;
        let job = RouterJob {
            chat: self.chat,
            input,
            revision,
            retried: false,
        };
        self.engine
            .finish_router(&job, &request, exchange)
            .await
            .unwrap()
            .decision
    }

    /// 별도 작업에서 도는 router 호출이 모두 돌아와 적용될 때까지 engine 루프 역할을 한다.
    pub(super) async fn settle(&mut self) {
        while self.is_judging() {
            let done = timeout(WAIT, self.engine.flow.router_rx.recv())
                .await
                .expect("router result should arrive in time")
                .expect("engine should keep the result channel open");
            self.engine.on_routed(done).await;
        }
    }

    pub(super) fn is_judging(&self) -> bool {
        !self.engine.flow.judging.is_empty()
    }

    /// 열려 있는 에이전트. 시험마다 하나뿐이다.
    pub(super) fn agent(&self) -> AgentId {
        *self
            .engine
            .flow
            .live
            .keys()
            .next()
            .expect("a session should be open")
    }

    pub(super) fn state(&self, input: InputId) -> InputState {
        self.engine
            .queue
            .input(input)
            .expect("input should be queued")
            .state
    }

    pub(super) fn record(&self, input: InputId) -> QueuedInput {
        self.engine
            .queue
            .input(input)
            .expect("input should be queued")
            .clone()
    }
}

/// 실행 중이 아닐 때 묻는 질문에 대한 router 답.
pub(super) fn idle_reply(keep_current: f64) -> FakeReply {
    ok(&answers(keep_current, None, None))
}

/// `target_model`까지 묻는 요청에 대한 router 답. `options`는 질문의 선택지 전체(마지막은 `other`)다.
pub(super) fn model_reply(keep_current: f64, options: &[&str], picked: &str) -> FakeReply {
    ok(&answers(keep_current, None, Some((options, picked))))
}

/// 실행 중일 때 묻는 질문에 대한 router 답. `relation`은 `RELATION_OPTIONS`, `send`는 `SEND_OPTIONS` 중 하나.
pub(super) fn running_reply(keep_current: f64, relation: &str, send: &str) -> FakeReply {
    ok(&answers(keep_current, Some((relation, send)), None))
}

fn answers(
    keep_current: f64,
    running: Option<(&str, &str)>,
    model: Option<(&[&str], &str)>,
) -> String {
    let mut answers = json!({
        "keep_current": { "type": "noul", "noul": keep_current },
        "is_actionable": { "type": "noul", "noul": 0.9 },
    });
    if let Some((options, picked)) = model {
        answers["target_model"] = choice(options, picked);
    }
    if let Some((relation, send)) = running {
        answers["relation_to_running"] = choice(&RELATION_OPTIONS, relation);
        answers["steer_or_spawn"] = choice(&SEND_OPTIONS, send);
    }
    json!({
        "model": "jev-1.13.0",
        "answers": answers,
        "usage": { "input_tokens": 10, "output_tokens": 2 },
    })
    .to_string()
}

fn choice(options: &[&str], picked: &str) -> Value {
    let probabilities: serde_json::Map<String, Value> = options
        .iter()
        .map(|option| {
            (
                option.to_owned().to_owned(),
                json!(f64::from(u8::from(*option == picked))),
            )
        })
        .collect();
    json!({ "type": "choice", "choice": picked, "probabilities": probabilities, "confidence": 1.0 })
}

/// 다시 보내지 않는 실패(키 거절)라 재시도 대기 없이 바로 판단 실패가 된다. 시계를 멈추면 기록 저장소 접근 시간 제한이 먼저 끝나므로 멈추지 않는다.
pub(super) fn router_down() -> Vec<FakeReply> {
    vec![key_rejected()]
}

pub(super) fn text(agent: AgentId, text: &str) -> ProviderEvent {
    ProviderEvent::Text {
        agent,
        subagent: None,
        text: text.to_owned(),
    }
}

pub(super) fn turn_completed(agent: AgentId) -> ProviderEvent {
    ProviderEvent::TurnCompleted {
        agent,
        origin: TurnOrigin::User,
    }
}

pub(super) fn context_size(agent: AgentId, tokens: u64) -> ProviderEvent {
    ProviderEvent::ContextSize {
        agent,
        tokens: Some(tokens),
    }
}

pub(super) fn subagent_started(agent: AgentId, id: &str, parent: Option<&str>) -> ProviderEvent {
    ProviderEvent::SubagentStarted {
        agent,
        subagent: SubagentId(id.to_owned()),
        parent: parent.map(|parent| SubagentId(parent.to_owned())),
    }
}

pub(super) fn subagent_ended(agent: AgentId, id: &str) -> ProviderEvent {
    ProviderEvent::SubagentEnded {
        agent,
        subagent: SubagentId(id.to_owned()),
    }
}

pub(super) fn tool_read(agent: AgentId, call_id: &str, path: &str) -> ProviderEvent {
    ProviderEvent::ToolCall {
        agent,
        subagent: None,
        call_id: call_id.to_owned(),
        activity: Activity::ReadingFile,
        detail: ToolDetail {
            category: ToolCategory::FileRead,
            paths: vec![path.to_owned()],
            read_lines: None,
            changed: None,
        },
    }
}

pub(super) fn tool_result(agent: AgentId, call_id: &str, output: &str) -> ProviderEvent {
    ProviderEvent::ToolResult {
        agent,
        subagent: None,
        call_id: call_id.to_owned(),
        output: output.to_owned(),
        exit_code: None,
    }
}

pub(super) fn permission(agent: AgentId, request_id: &str) -> ProviderEvent {
    ProviderEvent::PermissionRequested {
        agent,
        request_id: request_id.to_owned(),
        summary: "run cargo test".to_owned(),
        reason: "the build needs checking".to_owned(),
        call: None,
    }
}

/// 질문 하나를 가진 입력 요청.
pub(super) fn input_request(agent: AgentId, request_id: &str) -> ProviderEvent {
    ProviderEvent::InputRequested {
        agent,
        request_id: request_id.to_owned(),
        request: InputRequest {
            message: String::new(),
            fields: vec![InputField {
                id: "q".to_owned(),
                title: "Which?".to_owned(),
                description: String::new(),
                kind: InputFieldKind::Text,
                is_required: true,
                is_secret: false,
            }],
            url: None,
        },
    }
}

/// 규칙이 읽을 수 있는 호출을 가진 허가 요청.
pub(super) fn permission_for(
    agent: AgentId,
    request_id: &str,
    tool: PermissionTool,
    target: &str,
    paths: &[&str],
) -> ProviderEvent {
    ProviderEvent::PermissionRequested {
        agent,
        request_id: request_id.to_owned(),
        summary: format!("{target} {}", paths.join(" ")),
        reason: String::new(),
        call: Some(PermissionCall {
            tool,
            target: target.to_owned(),
            paths: paths.iter().map(|path| (*path).to_owned()).collect(),
        }),
    }
}
