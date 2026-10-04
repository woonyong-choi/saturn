//! 크래시 복구와 유휴 종료 테스트(#150): 끝나지 않은 실행은 보류로 되살리고 증명된 것만 자동으로 잇는다.
//! 마지막 TUI가 떠난 뒤 할 일이 없으면 유예 뒤 engine이 스스로 끝난다.

use saturn_core::sessions::memo::INTERRUPTED_RESULT;
use saturn_protocol::ids::{ChatId, TaskId, TaskLabel};
use saturn_protocol::rpc::ChatNotice;
use saturn_protocol::state::{EffectScope, SessionState, TaskState};

use super::support::{CLIENT, Flow, idle_reply, turn_completed};
use super::*;
use crate::providers::test_support::{Call, FakeProvider};
use crate::rpc::RpcEvent;

/// 크래시 뒤 같은 홈으로 다시 시작한 engine과 새 가짜 provider.
pub(super) struct Restarted {
    pub(super) engine: Engine,
    pub(super) fake: FakeProvider,
    pub(super) chat: ChatId,
    pub(super) fixture: Fixture,
    /// 다시 뜬 engine의 router가 받은 호출.
    pub(super) transport: Arc<FakeTransport>,
}

impl Restarted {
    /// 작업 중이던 `flow`의 engine을 정리 없이 버려 크래시를 흉내 낸 뒤 같은 홈으로 다시 시작하고, 크래시 복구를 한 번 돌린다.
    pub(super) async fn after_crash(flow: Flow, scope: EffectScope) -> Self {
        let Flow {
            engine,
            fixture,
            chat,
            ..
        } = flow;
        let run = engine.store.unfinished_runs().await.unwrap()[0].id;
        engine.store.set_effect_scope(run, scope).await.unwrap();
        drop(engine);
        Self::start_again(fixture, chat).await
    }

    /// 정상 종료한 것처럼 engine을 버리고 같은 홈으로 다시 시작해, 크래시 복구와 기록에 남긴 보류 복구를 한 번 돌린다.
    pub(super) async fn restart(self) -> Self {
        let Self {
            engine,
            chat,
            fixture,
            ..
        } = self;
        drop(engine);
        Self::start_again(fixture, chat).await
    }

    async fn start_again(fixture: Fixture, chat: ChatId) -> Self {
        Self::start_with_replies(fixture, chat, Vec::new()).await
    }

    /// `router_replies`는 다시 뜬 engine의 router가 시작 확인 뒤 차례로 낼 답이다.
    pub(super) async fn start_with_replies(
        fixture: Fixture,
        chat: ChatId,
        router_replies: Vec<FakeReply>,
    ) -> Self {
        let mut script = check_passes();
        script.extend(router_replies);
        let transport = FakeTransport::new(script);
        let env = fixture.env(true, Arc::clone(&transport)).await;
        let mut engine = fixture.start(env).await.unwrap();
        let fake = FakeProvider::new(crate::providers::test_support::CLAUDE);
        engine
            .flow
            .questions_of_connection
            .insert((chat, crate::providers::test_support::CLAUDE), true);
        engine.add_connection(chat, fake.connection());
        engine.recover_after_crash().await.unwrap();
        Self {
            engine,
            fake,
            chat,
            fixture,
            transport,
        }
    }

    pub(super) fn turns(&self) -> Vec<String> {
        self.fake
            .calls()
            .into_iter()
            .filter_map(|call| match call {
                Call::SendTurn { text, .. } => Some(text),
                _ => None,
            })
            .collect()
    }

    /// router 판단과 provider 응답을 기다리는 전달이 모두 끝날 때까지 engine 루프 역할을 한다.
    pub(super) async fn settle(&mut self) {
        while !self.engine.flow.deliveries.is_empty() || !self.engine.flow.judging.is_empty() {
            let flow = &mut self.engine.flow;
            tokio::select! {
                Some(done) = flow.router_rx.recv() => self.engine.on_routed(done).await,
                Some(message) = flow.provider_rx.recv() => self.engine.on_provider_msg(message).await,
                () = tokio::time::sleep(WAIT) => panic!("router and provider results should arrive in time"),
            }
        }
    }

    /// TUI가 소켓으로 붙을 때 받은 알림.
    pub(super) async fn attach(&mut self) -> Vec<Notification> {
        self.attach_client().await.1
    }

    /// `attach`와 같고 붙은 TUI도 돌려준다.
    pub(super) async fn attach_client(&mut self) -> (Client, Vec<Notification>) {
        let mut client = Client::connect(&self.fixture.socket()).await;
        let request = Request::Attach {
            chat: Some(self.chat),
            workdir: self.fixture.workdir.display().to_string(),
            env: Vec::new(),
            overrides: Vec::new(),
            add_dirs: Vec::new(),
        };
        let greeting = drive(&mut self.engine, async { client.attach(1, request).await }).await;
        (client, greeting)
    }
}

async fn crashed_while_working(scope: EffectScope) -> Restarted {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    Restarted::after_crash(flow, scope).await
}

pub(super) fn resume_suggested(seen: &[Notification]) -> Option<Vec<TaskLabel>> {
    seen.iter().find_map(|notification| match notification {
        Notification::ChatNotice {
            notice: ChatNotice::ResumeSuggested { held },
            ..
        } => Some(held.clone()),
        _ => None,
    })
}

// #150: 증명되지 않은 실행은 다시 보내지 않고 보류하며, 붙은 TUI에 `/continue`를 제안한다
#[tokio::test]
async fn unproven_run_is_held_not_resent_and_suggested_when_a_tui_attaches() {
    let mut restarted = crashed_while_working(EffectScope::NetworkPossible).await;

    assert!(restarted.fake.calls().is_empty());
    assert!(
        restarted
            .engine
            .store
            .unfinished_runs()
            .await
            .unwrap()
            .is_empty()
    );
    let session = restarted.engine.sessions.live_main(restarted.chat).unwrap();
    assert_eq!(session.state, SessionState::Held);
    let seen = restarted.attach().await;
    assert_eq!(resume_suggested(&seen), Some(vec![TaskLabel('A')]));
    assert!(seen.iter().any(|notification| matches!(
        notification,
        Notification::TaskChanged {
            label: TaskLabel('A'),
            state: TaskState::Held,
            ..
        }
    )));
}

// #150: 제안은 처음 붙은 TUI에만 한 번 보낸다
#[tokio::test]
async fn resume_suggestion_is_sent_only_to_the_first_tui() {
    let mut restarted = crashed_while_working(EffectScope::Unobserved).await;
    let first = restarted.attach().await;
    let second = restarted.attach().await;

    assert!(resume_suggested(&first).is_some());
    assert!(resume_suggested(&second).is_none());
}

// #150: 보류한 작업은 `/continue`로만 잇고, 같은 패킷 대신 파일 상태를 확인하게 하는 새 입력을 보낸다
#[tokio::test]
async fn continue_after_crash_resumes_the_session_with_a_state_check_input() {
    let mut restarted = crashed_while_working(EffectScope::NetworkPossible).await;
    let chat = restarted.chat;

    restarted.engine.continue_held(chat, None).await.unwrap();
    restarted.settle().await;

    let turns = restarted.turns();
    assert_eq!(turns.len(), 1);
    assert!(turns[0].starts_with(&format!(
        "Previous turn result (error): {INTERRUPTED_RESULT}\n"
    )));
    assert!(turns[0].ends_with("Request:\nfix the build"));
    assert!(restarted.fake.calls().iter().any(|call| matches!(
        call,
        Call::Open {
            resume: Some(_),
            ..
        }
    )));
}

// #150: 효과 범위가 증명된 실행은 사용자 개입 없이 새 입력으로 이어 가고 제안하지 않는다
#[tokio::test]
async fn proven_run_resumes_by_itself_with_a_state_check_input() {
    for scope in [
        EffectScope::ProvenByConfig,
        EffectScope::ProvenByObservation,
    ] {
        let mut restarted = crashed_while_working(scope).await;
        restarted.settle().await;

        let turns = restarted.turns();
        assert_eq!(turns.len(), 1, "{scope:?}");
        assert!(turns[0].starts_with("Previous turn result (error)"));
        assert!(turns[0].ends_with("Request:\nfix the build"));
        let seen = restarted.attach().await;
        assert!(resume_suggested(&seen).is_none(), "{scope:?}");
    }
}

// #150: 크래시 흔적이 없으면 아무것도 하지 않는다
#[tokio::test]
async fn start_without_unfinished_runs_recovers_nothing() {
    let fixture = Fixture::new();
    let mut engine = fixture.ready().await;

    engine.recover_after_crash().await.unwrap();

    assert!(engine.notices.resume_suggested.is_empty());
    assert!(engine.flow.held.is_empty());
}

fn short_grace(flow: &mut Flow) {
    flow.engine.idle_grace = Duration::from_millis(100);
}

async fn last_tui_leaves(flow: &mut Flow) {
    flow.engine
        .handle_event(RpcEvent::Disconnected(CLIENT))
        .await
        .unwrap();
}

// #150: 마지막 TUI가 떠난 뒤 할 일이 없으면 유예 뒤 engine이 스스로 끝난다
#[tokio::test]
async fn engine_ends_by_itself_after_the_grace_when_the_last_tui_left_with_no_work() {
    let mut flow = Flow::new(Vec::new()).await;
    short_grace(&mut flow);
    last_tui_leaves(&mut flow).await;

    let served = timeout(WAIT, flow.engine.serve()).await;

    assert!(matches!(served, Ok(Ok(()))));
}

// #150: 일이 남아 있으면 끝나지 않고, 일이 끝난 뒤에야 유예를 센다
#[tokio::test]
async fn engine_keeps_running_while_work_remains_and_ends_after_it_finishes() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    short_grace(&mut flow);
    last_tui_leaves(&mut flow).await;

    let still_running = timeout(Duration::from_millis(600), flow.engine.serve()).await;
    assert!(still_running.is_err());

    flow.claude_event(turn_completed(agent)).await;
    let served = timeout(WAIT, flow.engine.serve()).await;
    assert!(matches!(served, Ok(Ok(()))));
}

// #150: 유예 중 TUI가 붙으면 끝나지 않는다
#[tokio::test]
async fn engine_does_not_end_while_a_tui_is_attached() {
    let mut flow = Flow::new(Vec::new()).await;
    let (_client, _) = flow.attach().await;
    short_grace(&mut flow);

    let still_running = timeout(Duration::from_millis(600), flow.engine.serve()).await;

    assert!(still_running.is_err());
}

// #368: engine이 정상 종료했다 다시 떠도 크래시로 보류한 작업의 재개 제안이 처음 붙는 TUI에 다시 온다
#[tokio::test]
async fn held_task_is_suggested_again_after_the_engine_restarts() {
    let restarted = crashed_while_working(EffectScope::NetworkPossible).await;

    let mut restarted = restarted.restart().await;
    let seen = restarted.attach().await;

    assert_eq!(resume_suggested(&seen), Some(vec![TaskLabel('A')]));
    assert!(restarted.fake.calls().is_empty());
}

// #368: 다시 뜬 engine에서도 `/continue`는 같은 패킷 대신 파일 상태를 확인하게 하는 새 입력으로 이어 간다
#[tokio::test]
async fn continue_works_after_the_engine_restarts() {
    let mut restarted = crashed_while_working(EffectScope::NetworkPossible)
        .await
        .restart()
        .await;
    let chat = restarted.chat;

    restarted.engine.continue_held(chat, None).await.unwrap();
    restarted.settle().await;

    let turns = restarted.turns();
    assert_eq!(turns.len(), 1);
    assert!(turns[0].ends_with("Request:\nfix the build"));
}

// #368: 재개하거나 닫은 작업은 다시 뜬 engine에서 제안하지 않는다
#[tokio::test]
async fn resumed_or_closed_task_is_not_suggested_after_the_engine_restarts() {
    let mut resumed = crashed_while_working(EffectScope::NetworkPossible).await;
    let chat = resumed.chat;
    resumed.engine.continue_held(chat, None).await.unwrap();
    resumed.settle().await;
    let agent = *resumed.engine.flow.live.keys().next().unwrap();
    resumed
        .engine
        .on_provider_event(
            crate::providers::test_support::CLAUDE,
            turn_completed(agent),
        )
        .await
        .unwrap();
    let mut resumed = resumed.restart().await;
    assert!(resume_suggested(&resumed.attach().await).is_none());

    let mut closed = crashed_while_working(EffectScope::NetworkPossible).await;
    let chat = closed.chat;
    closed.engine.close_held(chat, TaskId(1)).await.unwrap();
    let mut closed = closed.restart().await;
    assert!(resume_suggested(&closed.attach().await).is_none());
    assert!(closed.engine.flow.held.is_empty());
}

// #368: `on_exit = "stop"`으로 보류한 작업도 engine이 다시 뜬 뒤 붙는 TUI에 재개를 제안한다
#[tokio::test]
async fn task_held_by_on_exit_stop_is_suggested_after_the_engine_restarts() {
    let mut flow = Flow::with_config("on_exit = \"stop\"\n", vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let (chat, agent) = (flow.chat, flow.agent());
    flow.engine
        .handle_event(RpcEvent::Disconnected(CLIENT))
        .await
        .unwrap();
    flow.engine.confirm_stopped_agent(chat, agent);
    flow.engine.check_stop_done(chat).await.unwrap();
    assert!(flow.engine.flow.stopping.is_empty());
    let Flow {
        engine, fixture, ..
    } = flow;
    drop(engine);
    let mut restarted = Restarted::start_again(fixture, chat).await;

    let seen = restarted.attach().await;

    assert_eq!(resume_suggested(&seen), Some(vec![TaskLabel('A')]));
}
