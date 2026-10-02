//! 입력 흐름 테스트 공용 도구: 소켓 없이 붙인 채팅, 가짜 provider, 가짜 judge 답.

use saturn_core::judges::{RELATION_OPTIONS, RouteDecision, SEND_OPTIONS};
use saturn_core::queue::{Permission, QueuedInput};
use saturn_protocol::ids::{AgentId, ChatId, InputId, Provider};
use saturn_protocol::state::InputState;
use serde_json::{Value, json};

use super::*;
use crate::Attachment;
use crate::chat_env::ChatEnv;
use crate::providers::ProviderConnection;
use crate::providers::test_support::FakeProvider;
use crate::rpc::ClientId;
use crate::store::NewInput;

pub(super) const CLIENT: ClientId = ClientId(1);

/// 소켓 없이 붙인 채팅 하나와, 그 채팅에 연결된 가짜 Claude를 가진 engine.
pub(super) struct Flow {
    pub(super) engine: Engine,
    pub(super) fake: FakeProvider,
    pub(super) transport: Arc<FakeTransport>,
    pub(super) chat: ChatId,
    pub(super) fixture: Fixture,
}

impl Flow {
    /// `judge_replies`는 시작 확인 뒤 judge가 차례로 낼 답이다.
    pub(super) async fn new(judge_replies: Vec<FakeReply>) -> Self {
        let fixture = Fixture::new();
        let mut script = check_passes();
        script.extend(judge_replies);
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

    /// judge 호출 수. 시작 확인의 두 호출은 뺀다.
    pub(super) fn judge_calls(&self) -> usize {
        self.transport.calls().len() - check_passes().len()
    }

    pub(super) async fn submit(&mut self, text: &str) -> InputId {
        self.submit_with(text, None, false).await
    }

    pub(super) async fn submit_with(
        &mut self,
        text: &str,
        pinned_model: Option<&str>,
        skip_relation: bool,
    ) -> InputId {
        let before = self.latest_input().await;
        self.engine
            .submit_input(
                CLIENT,
                self.chat,
                1,
                text.to_owned(),
                pinned_model.map(str::to_owned),
                skip_relation,
            )
            .await
            .unwrap();
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

    /// 지금 revision으로 judge에 한 번 묻고 그 판단을 돌려준다. 기록은 적용 때 쓴다.
    pub(super) async fn judge_now(&mut self, input: InputId, running: bool) -> RouteDecision {
        let record = self.record(input);
        let revision = self.engine.queue.revision(self.chat);
        self.engine
            .ask_judge(&record, running, revision)
            .await
            .unwrap()
            .decision
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

/// 실행 중이 아닐 때 묻는 질문에 대한 judge 답.
pub(super) fn idle_reply(keep_current: f64) -> FakeReply {
    ok(&answers(keep_current, None))
}

/// 실행 중일 때 묻는 질문에 대한 judge 답. `relation`은 `RELATION_OPTIONS`, `send`는 `SEND_OPTIONS` 중 하나.
pub(super) fn running_reply(keep_current: f64, relation: &str, send: &str) -> FakeReply {
    ok(&answers(keep_current, Some((relation, send))))
}

fn answers(keep_current: f64, running: Option<(&str, &str)>) -> String {
    let mut answers = json!({
        "keep_current": { "type": "noul", "noul": keep_current },
        "is_actionable": { "type": "noul", "noul": 0.9 },
    });
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
pub(super) fn judge_down() -> Vec<FakeReply> {
    vec![key_rejected()]
}
