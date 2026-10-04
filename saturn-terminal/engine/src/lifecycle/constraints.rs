//! 제약 등록 테스트: router `is_constraint`로 등록 후보를 정하고, 자동 등록, 묻기, 미등록, `full` 모드, 입력 취소를 확인한다.
//! 설계: docs/design/constraints.md

use saturn_core::constraints::split_sentences;
use saturn_protocol::ids::{ChatId, ConstraintAskId};
use saturn_protocol::rpc::{ChatNotice, ConstraintAskAnswer, Notification};
use saturn_protocol::state::{Disposition, InputState};

use super::Client;
use super::support::{
    CLIENT, Flow, constraint_reply, idle_reply, lines_reply, retry_reply, router_down,
    running_constraint_reply,
};
use crate::store::{
    Actor, ConstraintState, EventKind, EventReason, HistoryEntry, StoredConstraint,
};
use crate::{Engine, EngineError};

const RULE: &str = "에러 메시지는 영어로 통일해";

const LONG_INPUT: &str = "Always write error messages in English, never in Korean or any other language you might guess. \
    Please fix the failing build in the auth module before the end of the day today. \
    Do not use unwrap anywhere in the code you touch, please use proper error handling instead.";

async fn constraints(flow: &Flow) -> Vec<StoredConstraint> {
    flow.engine
        .store
        .constraints_of_chat(flow.chat)
        .await
        .unwrap()
}

async fn events(
    flow: &Flow,
) -> Vec<(
    EventKind,
    Actor,
    Option<EventReason>,
    Option<saturn_protocol::ids::JudgmentId>,
)> {
    flow.engine
        .store
        .constraint_events_of_chat(flow.chat)
        .await
        .unwrap()
}

/// 대화 기록의 제약 줄. 채팅을 다시 열어도 같은 줄이 나오는지 보려고 기록 저장소에서 읽는다.
async fn history_lines(flow: &Flow) -> Vec<(EventKind, Option<EventReason>, String)> {
    let page = flow
        .engine
        .store
        .history_page(flow.chat, None, 50)
        .await
        .unwrap();
    page.entries
        .into_iter()
        .filter_map(|entry| match entry {
            HistoryEntry::Constraint { kind, reason, rule } => Some((kind, reason, rule)),
            _ => None,
        })
        .collect()
}

fn added_notices(notifications: &[Notification]) -> Vec<(String, bool)> {
    notifications
        .iter()
        .filter_map(|notification| match notification {
            Notification::ChatNotice {
                notice: ChatNotice::ConstraintAdded { rule, unconfirmed },
                ..
            } => Some((rule.clone(), *unconfirmed)),
            _ => None,
        })
        .collect()
}

fn asked(notifications: &[Notification]) -> Vec<(ConstraintAskId, String)> {
    notifications
        .iter()
        .filter_map(|notification| match notification {
            Notification::ConstraintAsked { ask, rule, .. } => Some((*ask, rule.clone())),
            _ => None,
        })
        .collect()
}

async fn answer(
    engine: &mut Engine,
    ask: ConstraintAskId,
    answer: ConstraintAskAnswer,
) -> Result<(), EngineError> {
    engine.answer_constraint_ask(CLIENT, ask, answer).await
}

#[tokio::test]
async fn constraint_at_or_above_the_auto_threshold_is_registered_with_a_chat_line() {
    let mut flow = Flow::new(vec![constraint_reply(0.95, 0.85)]).await;
    let mut client = flow.client().await;

    let input = flow.submit(RULE).await;

    let stored = constraints(&flow).await;
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].rule, RULE);
    assert_eq!(stored[0].input, input);
    assert_eq!(stored[0].state, ConstraintState::Active);
    assert!(stored[0].scope.is_empty());
    let recorded = events(&flow).await;
    assert_eq!(recorded.len(), 1);
    assert_eq!(
        (recorded[0].0, recorded[0].1, recorded[0].2),
        (EventKind::Added, Actor::Router, None)
    );
    assert!(
        recorded[0].3.is_some(),
        "the event should point at the judgment"
    );
    assert_eq!(
        history_lines(&flow).await,
        vec![(EventKind::Added, None, RULE.to_owned())]
    );
    let seen = client.window().await;
    assert_eq!(added_notices(&seen), vec![(RULE.to_owned(), false)]);
    assert!(asked(&seen).is_empty());
    assert_eq!(flow.state(input), InputState::Applied);
}

#[tokio::test]
async fn constraint_at_the_exact_auto_threshold_is_registered() {
    let mut flow = Flow::new(vec![constraint_reply(0.95, 0.8)]).await;

    flow.submit(RULE).await;

    assert_eq!(constraints(&flow).await[0].state, ConstraintState::Active);
}

#[tokio::test]
async fn constraint_scope_comes_from_paths_in_the_rule() {
    let rule = "src/auth/ 안에서는 unwrap 쓰지 마";
    let mut flow = Flow::new(vec![constraint_reply(0.95, 0.9)]).await;

    flow.submit(rule).await;

    let stored = constraints(&flow).await;
    assert_eq!(stored[0].rule, rule);
    assert_eq!(stored[0].scope, vec!["src/auth/".to_owned()]);
}

#[tokio::test]
async fn constraint_in_the_ask_band_is_stored_as_candidate_and_asked() {
    let mut flow = Flow::new(vec![constraint_reply(0.95, 0.75)]).await;
    let mut client = flow.client().await;

    let input = flow.submit("테스트 메시지는 한국어로 둬").await;

    let stored = constraints(&flow).await;
    assert_eq!(stored[0].state, ConstraintState::Candidate);
    assert!(
        events(&flow).await.is_empty(),
        "no line before the user answers"
    );
    assert!(history_lines(&flow).await.is_empty());
    let seen = client.window().await;
    assert_eq!(asked(&seen).len(), 1);
    assert_eq!(asked(&seen)[0].1, "테스트 메시지는 한국어로 둬");
    assert!(added_notices(&seen).is_empty());
    assert_eq!(
        flow.state(input),
        InputState::Applied,
        "the input is not held back by the question"
    );
    let open = flow.engine.store.open_constraint_asks().await.unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(
        open[0].rules,
        vec!["테스트 메시지는 한국어로 둬".to_owned()]
    );
}

#[tokio::test]
async fn constraint_ask_answered_yes_registers_and_leaves_a_line() {
    let mut flow = Flow::new(vec![constraint_reply(0.95, 0.75)]).await;
    let mut client = flow.client().await;
    flow.submit("테스트 메시지는 한국어로 둬").await;
    let (ask, _) = asked(&client.window().await)[0].clone();

    answer(&mut flow.engine, ask, ConstraintAskAnswer::Yes)
        .await
        .unwrap();

    assert_eq!(constraints(&flow).await[0].state, ConstraintState::Active);
    let recorded = events(&flow).await;
    assert_eq!(
        (recorded[0].0, recorded[0].1),
        (EventKind::Added, Actor::User)
    );
    assert_eq!(
        history_lines(&flow).await,
        vec![(
            EventKind::Added,
            None,
            "테스트 메시지는 한국어로 둬".to_owned()
        )]
    );
    assert_eq!(
        added_notices(&client.window().await),
        vec![("테스트 메시지는 한국어로 둬".to_owned(), false)]
    );
    assert!(
        flow.engine
            .store
            .open_constraint_asks()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn constraint_ask_answered_no_releases_it_without_a_chat_line() {
    let mut flow = Flow::new(vec![constraint_reply(0.95, 0.75)]).await;
    let mut client = flow.client().await;
    flow.submit("테스트 메시지는 한국어로 둬").await;
    let (ask, _) = asked(&client.window().await)[0].clone();

    answer(&mut flow.engine, ask, ConstraintAskAnswer::No)
        .await
        .unwrap();

    assert_eq!(constraints(&flow).await[0].state, ConstraintState::Released);
    let recorded = events(&flow).await;
    assert_eq!(
        (recorded[0].0, recorded[0].1, recorded[0].2),
        (
            EventKind::Released,
            Actor::User,
            Some(EventReason::Declined)
        )
    );
    assert!(history_lines(&flow).await.is_empty());
    assert!(added_notices(&client.window().await).is_empty());
}

#[tokio::test]
async fn constraint_ask_answer_is_kept_on_the_judgment_and_a_late_answer_is_rejected() {
    let mut flow = Flow::new(vec![constraint_reply(0.95, 0.75)]).await;
    let mut client = flow.client().await;
    flow.submit("테스트 메시지는 한국어로 둬").await;
    let (ask, _) = asked(&client.window().await)[0].clone();

    answer(&mut flow.engine, ask, ConstraintAskAnswer::No)
        .await
        .unwrap();
    let late = answer(&mut flow.engine, ask, ConstraintAskAnswer::Yes).await;

    assert!(matches!(late, Err(EngineError::UnexpectedAnswer { .. })));
    assert_eq!(constraints(&flow).await[0].state, ConstraintState::Released);
    let asked_answers = flow.engine.store.asked_judgments().await.unwrap();
    assert_eq!(
        asked_answers,
        vec![(
            1.0,
            Some(saturn_core::routers::calibration::AskedAnswer::Wrong)
        )],
        "the router leaned to register, so declining contradicts it"
    );
}

#[tokio::test]
async fn constraint_below_the_ask_threshold_is_not_registered() {
    let mut flow = Flow::new(vec![constraint_reply(0.95, 0.65)]).await;
    let mut client = flow.client().await;

    flow.submit("이 버그 좀 고쳐줘").await;

    assert!(constraints(&flow).await.is_empty());
    assert!(events(&flow).await.is_empty());
    let seen = client.window().await;
    assert!(added_notices(&seen).is_empty());
    assert!(asked(&seen).is_empty());
}

#[tokio::test]
async fn constraint_in_full_mode_registers_in_the_ask_band_without_asking_and_marks_it() {
    let mut flow = Flow::with_config(
        "permission.mode = \"full\"\n",
        vec![constraint_reply(0.95, 0.75)],
    )
    .await;
    let mut client = flow.client().await;

    flow.submit("테스트 메시지는 한국어로 둬").await;

    assert_eq!(constraints(&flow).await[0].state, ConstraintState::Active);
    let recorded = events(&flow).await;
    assert_eq!(
        (recorded[0].0, recorded[0].1, recorded[0].2),
        (
            EventKind::Added,
            Actor::Router,
            Some(EventReason::Unconfirmed)
        )
    );
    assert_eq!(
        history_lines(&flow).await,
        vec![(
            EventKind::Added,
            Some(EventReason::Unconfirmed),
            "테스트 메시지는 한국어로 둬".to_owned()
        )]
    );
    let seen = client.window().await;
    assert_eq!(
        added_notices(&seen),
        vec![("테스트 메시지는 한국어로 둬".to_owned(), true)]
    );
    assert!(asked(&seen).is_empty());
    assert!(
        flow.engine
            .store
            .open_constraint_asks()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn constraint_in_full_mode_above_the_auto_threshold_has_no_unconfirmed_mark() {
    let mut flow = Flow::with_config(
        "permission.mode = \"full\"\n",
        vec![constraint_reply(0.95, 0.9)],
    )
    .await;
    let mut client = flow.client().await;

    flow.submit(RULE).await;

    assert_eq!(
        added_notices(&client.window().await),
        vec![(RULE.to_owned(), false)]
    );
    assert_eq!(events(&flow).await[0].2, None);
}

#[tokio::test]
async fn constraint_is_not_registered_when_the_router_fails() {
    let mut flow = Flow::new(router_down()).await;
    let mut client = flow.client().await;

    let input = flow.submit(RULE).await;

    assert!(constraints(&flow).await.is_empty());
    assert!(events(&flow).await.is_empty());
    assert!(added_notices(&client.window().await).is_empty());
    assert_eq!(
        flow.state(input),
        InputState::Applied,
        "the input is still handled"
    );
}

#[tokio::test]
async fn constraint_is_not_registered_when_the_answer_is_missing() {
    let without_constraint = retry_reply(0.95);
    let mut flow = Flow::new(vec![without_constraint]).await;

    let input = flow.submit(RULE).await;

    assert!(constraints(&flow).await.is_empty());
    assert_eq!(flow.state(input), InputState::Applied);
}

#[tokio::test]
async fn constraint_input_without_the_router_is_never_judged() {
    let mut flow = Flow::new(Vec::new()).await;

    flow.submit_with(RULE, None, true).await;

    assert_eq!(flow.router_calls(), 0);
    assert!(constraints(&flow).await.is_empty());
}

#[tokio::test]
async fn constraint_of_an_input_canceled_while_judging_is_not_registered() {
    let mut flow = Flow::new(vec![constraint_reply(0.95, 0.9)]).await;
    let release = flow.transport.hold_next_call();
    flow.engine
        .submit_input(CLIENT, flow.chat, 1, RULE.to_owned(), false)
        .await
        .unwrap();
    let (input, _) = flow.engine.queue.next_to_route(flow.chat).unwrap();
    assert!(flow.is_judging());

    flow.engine.cancel_input(CLIENT, input).await.unwrap();
    release.notify_one();
    flow.settle().await;

    assert_eq!(flow.state(input), InputState::Cancelled);
    assert!(constraints(&flow).await.is_empty());
    assert!(events(&flow).await.is_empty());
    assert!(flow.fake.calls().is_empty());
}

#[tokio::test]
async fn constraint_of_an_input_canceled_after_registration_is_released_with_a_line() {
    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        running_constraint_reply(0.95, "continues", "queue", 0.9),
    ])
    .await;
    let mut client = flow.client().await;
    flow.submit("first request").await;
    let second = flow.submit(RULE).await;
    assert_eq!(flow.state(second), InputState::Queued);
    assert_eq!(constraints(&flow).await[0].state, ConstraintState::Active);

    flow.engine.cancel_input(CLIENT, second).await.unwrap();

    assert_eq!(constraints(&flow).await[0].state, ConstraintState::Released);
    let recorded = events(&flow).await;
    assert_eq!(recorded.len(), 2);
    assert_eq!(
        (recorded[1].0, recorded[1].2),
        (EventKind::Released, Some(EventReason::InputCanceled))
    );
    assert_eq!(
        history_lines(&flow).await,
        vec![
            (EventKind::Added, None, RULE.to_owned()),
            (
                EventKind::Released,
                Some(EventReason::InputCanceled),
                RULE.to_owned()
            ),
        ]
    );
    let released: Vec<String> = client
        .window()
        .await
        .into_iter()
        .filter_map(|notification| match notification {
            Notification::ChatNotice {
                notice: ChatNotice::ConstraintReleased { rule },
                ..
            } => Some(rule),
            _ => None,
        })
        .collect();
    assert_eq!(released, vec![RULE.to_owned()]);
}

#[tokio::test]
async fn constraint_ask_is_closed_when_its_input_is_canceled() {
    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        running_constraint_reply(0.95, "continues", "queue", 0.75),
    ])
    .await;
    let mut client = flow.client().await;
    flow.submit("first request").await;
    let second = flow.submit("테스트 메시지는 한국어로 둬").await;
    let (ask, _) = asked(&client.window().await)[0].clone();

    flow.engine.cancel_input(CLIENT, second).await.unwrap();

    assert!(
        flow.engine
            .store
            .open_constraint_asks()
            .await
            .unwrap()
            .is_empty()
    );
    let resolved = client
        .window()
        .await
        .into_iter()
        .any(|notification| matches!(notification, Notification::ConstraintAskResolved { ask: closed } if closed == ask));
    assert!(resolved, "the window should be withdrawn from every TUI");
    let late = answer(&mut flow.engine, ask, ConstraintAskAnswer::Yes).await;
    assert!(matches!(late, Err(EngineError::UnexpectedAnswer { .. })));
}

#[tokio::test]
async fn constraint_is_registered_once_when_the_chat_revision_changes_during_judgment() {
    let mut flow = Flow::new(vec![constraint_reply(0.95, 0.9), retry_reply(0.95)]).await;
    let input = flow.accept_only(RULE).await;
    let other = flow.accept_only("other").await;
    let stale = flow.router_now(input, false).await;
    // 다른 입력의 판단이 먼저 적용돼 채팅 revision이 오른다
    let revision = flow.engine.queue.revision(flow.chat);
    let applied = saturn_core::routers::RouteDecision {
        revision,
        settings: flow.engine.settings.current().unwrap(),
        disposition: Disposition::Queue,
        is_conflict: false,
        keep_current: true,
        model: None,
        resume_held: false,
        fallbacks: Vec::new(),
    };
    flow.engine
        .apply_decision(other, applied, false)
        .await
        .unwrap();

    flow.engine
        .apply_decision(input, stale, false)
        .await
        .unwrap();
    flow.settle().await;

    let stored = constraints(&flow).await;
    assert_eq!(stored.len(), 1, "registered exactly once");
    assert_eq!(stored[0].rule, RULE);
    assert_eq!(stored[0].state, ConstraintState::Active);
    assert_eq!(
        flow.router_calls(),
        2,
        "the input handling was judged again"
    );
    let retried = flow
        .transport
        .calls()
        .into_iter()
        .filter_map(|call| call.2)
        .next_back()
        .unwrap();
    assert!(
        !retried.contains("is_constraint"),
        "the second request should not ask again"
    );
}

#[tokio::test]
async fn constraint_long_input_registers_only_the_sentences_the_router_calls_constraints() {
    assert!(split_sentences(LONG_INPUT).is_some_and(|sentences| sentences.len() == 3));
    let mut flow = Flow::new(vec![
        constraint_reply(0.95, 0.9),
        lines_reply(&[0.9, 0.1, 0.8]),
    ])
    .await;
    let mut client = flow.client().await;

    flow.submit(LONG_INPUT).await;

    let stored = constraints(&flow).await;
    let rules: Vec<(u32, &str)> = stored
        .iter()
        .map(|constraint| (constraint.line, constraint.rule.as_str()))
        .collect();
    assert_eq!(
        rules,
        vec![
            (
                1,
                "Always write error messages in English, never in Korean or any other language you might guess."
            ),
            (
                3,
                "Do not use unwrap anywhere in the code you touch, please use proper error handling instead."
            ),
        ]
    );
    assert!(
        stored
            .iter()
            .all(|constraint| LONG_INPUT.contains(&constraint.rule))
    );
    assert_eq!(added_notices(&client.window().await).len(), 2);
    assert_eq!(flow.router_calls(), 2);
}

#[tokio::test]
async fn constraint_long_input_registers_the_whole_text_when_the_split_question_fails() {
    let mut flow = Flow::new(vec![constraint_reply(0.95, 0.9), super::key_rejected()]).await;

    flow.submit(LONG_INPUT).await;

    let stored = constraints(&flow).await;
    assert_eq!(stored.len(), 1);
    assert_eq!((stored[0].line, stored[0].rule.as_str()), (0, LONG_INPUT));
}

#[tokio::test]
async fn constraint_long_input_in_the_ask_band_asks_once_for_all_rules() {
    let mut flow = Flow::new(vec![
        constraint_reply(0.95, 0.75),
        lines_reply(&[0.9, 0.1, 0.8]),
    ])
    .await;
    let mut client = flow.client().await;
    flow.submit(LONG_INPUT).await;
    let seen = client.window().await;
    let asks = asked(&seen);
    assert_eq!(asks.len(), 1, "one window per input");
    assert_eq!(asks[0].1.lines().count(), 2);

    answer(&mut flow.engine, asks[0].0, ConstraintAskAnswer::Yes)
        .await
        .unwrap();

    let states: Vec<ConstraintState> = constraints(&flow)
        .await
        .iter()
        .map(|constraint| constraint.state)
        .collect();
    assert_eq!(
        states,
        vec![ConstraintState::Active, ConstraintState::Active]
    );
    assert_eq!(history_lines(&flow).await.len(), 2);
}

#[tokio::test]
async fn constraint_ask_reaches_a_tui_that_attaches_later_and_after_a_restart() {
    let mut flow = Flow::new(vec![constraint_reply(0.95, 0.75)]).await;
    flow.submit("테스트 메시지는 한국어로 둬").await;

    let (_client, greeting): (Client, Vec<Notification>) = flow.attach().await;

    assert_eq!(asked(&greeting).len(), 1);
    // engine을 다시 켠 뒤에는 기록 저장소의 열린 확인이 같은 알림으로 돌아온다
    let mut restarted = Flow::new(Vec::new()).await;
    let chat = restarted.chat;
    stored_candidate(&mut restarted, chat).await;
    restarted.engine.restore_constraint_asks().await.unwrap();
    let (_other, greeting) = restarted.attach().await;
    assert_eq!(asked(&greeting).len(), 1);
}

/// 열린 확인이 하나 있는 기록 저장소.
async fn stored_candidate(flow: &mut Flow, chat: ChatId) {
    let input = flow.accept_only("keep it short").await;
    flow.engine
        .store
        .register_constraints(&crate::store::NewRegistration {
            chat,
            input,
            rules: &[crate::store::NewRule {
                line: 0,
                rule: "keep it short".to_owned(),
                scope: Vec::new(),
            }],
            state: ConstraintState::Candidate,
            actor: Actor::Router,
            reason: None,
            judgment: None,
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn constraint_thresholds_are_read_from_settings() {
    let mut flow = Flow::with_config(
        "[router.thresholds]\nis_constraint = 0.9\nconstraint_ask = 0.6\n",
        vec![constraint_reply(0.95, 0.85), constraint_reply(0.95, 0.65)],
    )
    .await;
    let mut client = flow.client().await;

    flow.submit(RULE).await;
    flow.engine
        .finish_task(flow.chat, flow.agent())
        .await
        .unwrap();
    flow.settle().await;
    flow.submit("다른 규칙도 지켜줘").await;

    let states: Vec<ConstraintState> = constraints(&flow)
        .await
        .iter()
        .map(|constraint| constraint.state)
        .collect();
    assert_eq!(
        states,
        vec![ConstraintState::Candidate, ConstraintState::Candidate]
    );
    assert_eq!(asked(&client.window().await).len(), 2);
}
