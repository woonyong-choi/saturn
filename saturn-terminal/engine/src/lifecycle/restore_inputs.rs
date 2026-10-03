//! 재시작 뒤 입력 복원 테스트(#370): 기록 저장소에 접수만 되고 보내지 않은 입력이 접수 순서대로 대기열에 돌아와
//! 상태별 규칙에 따라 처리된다.

use saturn_protocol::ids::{InputId, Provider, TaskLabel};
use saturn_protocol::state::{EffectScope, InputState};

use super::crash_recovery::{Restarted, resume_suggested};
use super::support::{Flow, idle_reply, running_reply, turn_completed};
use crate::providers::test_support::Call;

fn turn_texts(restarted: &Restarted) -> Vec<String> {
    restarted
        .turns()
        .into_iter()
        .map(|text| text.lines().last().unwrap_or_default().to_owned())
        .collect()
}

async fn complete_turn(restarted: &mut Restarted) {
    let agent = *restarted.engine.flow.live.keys().next().unwrap();
    restarted
        .engine
        .on_provider_event(Provider::Claude, turn_completed(agent))
        .await
        .unwrap();
    restarted.settle().await;
}

async fn set_state(flow: &Flow, input: InputId, state: InputState) {
    flow.engine
        .store
        .set_input_state(input, state, None)
        .await
        .unwrap();
}

fn state_of(restarted: &Restarted, input: InputId) -> InputState {
    restarted.engine.queue.input(input).unwrap().state
}

// 크래시: 실행 중이던 작업을 보류하면 그 뒤 대기하던 입력과 판단 중이던 입력도 멈춤과 같게 접수 순서대로 보류로 돌아오고,
// 보내지 않은 채 `/continue`를 기다렸다가 재개하면 차례로 보내진다
#[tokio::test]
async fn unsent_inputs_are_held_with_the_crashed_task_and_resume_in_order() {
    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        running_reply(0.95, "continues", "queue"),
    ])
    .await;
    flow.submit("first").await;
    let waiting = flow.submit("second").await;
    assert_eq!(flow.state(waiting), InputState::Queued);
    let judging = flow.accept_only("third").await;
    let mut restarted = {
        let Flow {
            engine,
            fixture,
            chat,
            ..
        } = flow;
        let run = engine.store.unfinished_runs().await.unwrap()[0].id;
        engine
            .store
            .set_effect_scope(run, EffectScope::NetworkPossible)
            .await
            .unwrap();
        drop(engine);
        Restarted::start_with_replies(fixture, chat, Vec::new()).await
    };
    restarted.settle().await;

    assert!(restarted.fake.calls().is_empty());
    assert_eq!(state_of(&restarted, waiting), InputState::Held);
    assert_eq!(state_of(&restarted, judging), InputState::Held);
    let seen = restarted.attach().await;
    assert_eq!(resume_suggested(&seen), Some(vec![TaskLabel('A')]));

    let chat = restarted.chat;
    restarted.engine.continue_held(chat, None).await.unwrap();
    restarted.settle().await;
    for _ in 0..3 {
        complete_turn(&mut restarted).await;
    }
    let texts = turn_texts(&restarted);
    let position = |wanted: &str| texts.iter().position(|text| text == wanted).unwrap();
    assert!(position("second") < position("third"), "{texts:?}");
    assert_eq!(texts.iter().filter(|text| *text == "first").count(), 1);
}

// 정상 종료: 접수만 되고 보내지 않은 대기 입력은 다시 뜬 engine이 접수 순서대로 보낸다
#[tokio::test]
async fn waiting_inputs_are_sent_in_accept_order_after_a_clean_restart() {
    let mut flow = Flow::new(Vec::new()).await;
    for text in ["first", "second"] {
        let input = flow.accept_only(text).await;
        set_state(&flow, input, InputState::Queued).await;
    }
    let (fixture, chat) = (flow.fixture, flow.chat);
    drop(flow.engine);

    let mut restarted = Restarted::start_with_replies(fixture, chat, Vec::new()).await;
    restarted.settle().await;

    assert_eq!(turn_texts(&restarted), ["first"]);
    complete_turn(&mut restarted).await;
    assert_eq!(turn_texts(&restarted), ["first", "second"]);
}

// 판단 중이던 입력은 다시 판단한 뒤 보낸다
#[tokio::test]
async fn judging_input_is_judged_again_and_sent_after_a_restart() {
    let mut flow = Flow::new(Vec::new()).await;
    flow.accept_only("judge me").await;
    let (fixture, chat) = (flow.fixture, flow.chat);
    drop(flow.engine);

    let mut restarted = Restarted::start_with_replies(fixture, chat, vec![idle_reply(0.95)]).await;
    restarted.settle().await;

    assert_eq!(turn_texts(&restarted), ["judge me"]);
}

// 보류 입력은 보류로 남아 보내지 않고, 처음 붙는 TUI에 재개를 제안하며, `/continue`로만 보낸다
#[tokio::test]
async fn held_input_stays_held_after_restart_until_continue() {
    let mut flow = Flow::new(Vec::new()).await;
    let held = flow.accept_only("held").await;
    let chat = flow.chat;
    flow.engine.stop_chat(chat).await.unwrap();
    assert_eq!(flow.state(held), InputState::Held);
    let (fixture, chat) = (flow.fixture, flow.chat);
    drop(flow.engine);

    let mut restarted = Restarted::start_with_replies(fixture, chat, Vec::new()).await;
    restarted.settle().await;

    assert_eq!(state_of(&restarted, held), InputState::Held);
    assert!(restarted.fake.calls().is_empty());
    let seen = restarted.attach().await;
    assert_eq!(resume_suggested(&seen), Some(vec![TaskLabel('A')]));

    restarted.engine.continue_held(chat, None).await.unwrap();
    restarted.settle().await;
    assert_eq!(turn_texts(&restarted), ["held"]);
}

// 보냈는지 모르는 `전달 중` 입력은 다시 보내지 않고 `전달 중`으로 남는다
#[tokio::test]
async fn delivering_input_is_never_resent_after_restart() {
    let mut flow = Flow::new(Vec::new()).await;
    let unknown = flow.accept_only("maybe sent").await;
    set_state(&flow, unknown, InputState::Delivering).await;
    let (fixture, chat) = (flow.fixture, flow.chat);
    drop(flow.engine);

    let mut restarted = Restarted::start_with_replies(fixture, chat, Vec::new()).await;
    restarted.settle().await;

    assert_eq!(state_of(&restarted, unknown), InputState::Delivering);
    assert!(
        !restarted
            .fake
            .calls()
            .iter()
            .any(|call| matches!(call, Call::SendTurn { .. } | Call::Open { .. }))
    );
}
