//! 제약 해제·예외 테스트: router 제안과 사용자 명령의 구별, 이번 작업 예외의 수명, 자동 적용 정책, 낡은 판단과 취소.
//! 설계: docs/design/constraints.md#해제와-예외-판단

use saturn_protocol::ids::{ConstraintId, TaskId};
use saturn_protocol::rpc::{ChatNotice, Notification};

use super::FakeReply;
use super::crash_recovery::Restarted;
use super::support::{
    CLIENT, Flow, change_distribution_reply, change_reply, idle_reply, running_constraint_reply,
};
use crate::EngineError;
use crate::store::{
    Actor, ChangeOutcome, ConstraintChange, ConstraintState, EventKind, EventReason, ExceptionKind,
    NewChange, NewRegistration, NewRule, StoredConstraint,
};

const ON: &str = "constraint.auto_apply = true\n";
const RULE: &str = "에러 메시지는 영어로 통일해";
const ASK: &str = "그 규칙 이번만 풀고 진행해";

/// 첫 입력으로 작업을 열고 그 입력에서 나온 유효 제약 하나를 직접 저장한다. 자동 등록 경로와 무관하게 쓴다.
async fn with_constraint(config: &str, mut replies: Vec<FakeReply>) -> (Flow, ConstraintId) {
    replies.insert(0, idle_reply(0.95));
    let mut flow = Flow::with_config(config, replies).await;
    flow.fake.verify_steer();
    let first = flow.submit("first request").await;
    let registered = flow
        .engine
        .store
        .register_constraints(&NewRegistration {
            chat: flow.chat,
            input: first,
            rules: &[NewRule {
                line: 0,
                rule: RULE.to_owned(),
                scope: Vec::new(),
            }],
            state: ConstraintState::Active,
            actor: Actor::User,
            reason: None,
            judgment: None,
        })
        .await
        .unwrap();
    (flow, registered.constraints[0])
}

/// 실행 중인 작업에 끼워 넣어 그 작업에 속하는 입력의 router 답.
fn steer_reply() -> FakeReply {
    running_constraint_reply(0.95, "refines", "steer", 0.1)
}

async fn constraints(flow: &Flow) -> Vec<StoredConstraint> {
    flow.engine
        .store
        .constraints_of_chat(flow.chat)
        .await
        .unwrap()
}

async fn events(flow: &Flow) -> Vec<(EventKind, Actor, Option<EventReason>, bool)> {
    flow.engine
        .store
        .constraint_events_of_chat(flow.chat)
        .await
        .unwrap()
        .into_iter()
        .map(|(kind, actor, reason, judgment)| (kind, actor, reason, judgment.is_some()))
        .collect()
}

fn notices(notifications: Vec<Notification>) -> Vec<ChatNotice> {
    notifications
        .into_iter()
        .filter_map(|notification| match notification {
            Notification::ChatNotice { notice, .. } => Some(notice),
            _ => None,
        })
        .filter(|notice| {
            matches!(
                notice,
                ChatNotice::ConstraintReleased { .. }
                    | ChatNotice::ConstraintPaused { .. }
                    | ChatNotice::ConstraintExcepted { .. }
                    | ChatNotice::ConstraintResumed { .. }
            )
        })
        .collect()
}

#[tokio::test]
async fn router_change_is_applied_by_kind_and_marked_as_the_router() {
    let table = [
        ("release_1", Some(ConstraintState::Released), None),
        (
            "once_1",
            Some(ConstraintState::Active),
            Some(ExceptionKind::Once),
        ),
        (
            "scoped_1",
            Some(ConstraintState::Active),
            Some(ExceptionKind::Scoped),
        ),
        ("none", Some(ConstraintState::Active), None),
    ];
    for (picked, state, exception) in table {
        let (mut flow, _) = with_constraint(ON, vec![steer_reply(), change_reply(1, picked)]).await;
        let mut client = flow.client().await;

        let second = flow.submit(ASK).await;

        assert!(flow.record(second).task.is_some(), "{picked}");
        let stored = constraints(&flow).await;
        assert_eq!(Some(stored[0].state), state, "{picked}");
        assert_eq!(
            stored[0].exception.as_ref().map(|found| found.kind),
            exception,
            "{picked}"
        );
        if exception == Some(ExceptionKind::Scoped) {
            // 조건 문장은 생성하지 않고 입력 원문의 연속된 글을 그대로 둔다
            let condition = stored[0].exception.as_ref().unwrap().condition.clone();
            assert_eq!(condition.as_deref(), Some(ASK));
        }
        let recorded = events(&flow).await;
        if picked == "none" {
            assert_eq!(recorded.len(), 1, "only the registration");
            assert!(notices(client.window().await).is_empty());
        } else {
            assert_eq!(recorded.len(), 2, "{picked}");
            assert_eq!(
                (recorded[1].1, recorded[1].3),
                (Actor::Router, true),
                "{picked}"
            );
            assert_eq!(notices(client.window().await).len(), 1, "{picked}");
        }
    }
}

#[tokio::test]
async fn task_exception_ends_with_its_task_and_scoped_exception_stays() {
    let (mut flow, _) = with_constraint(ON, vec![steer_reply(), change_reply(1, "once_1")]).await;
    let mut client = flow.client().await;
    flow.submit(ASK).await;
    assert!(constraints(&flow).await[0].exception.is_some());
    client.window().await;

    flow.engine
        .finish_task(flow.chat, flow.agent())
        .await
        .unwrap();

    let stored = constraints(&flow).await;
    assert_eq!(stored[0].state, ConstraintState::Active);
    assert_eq!(stored[0].exception, None);
    let kinds: Vec<(EventKind, Actor)> = events(&flow)
        .await
        .into_iter()
        .map(|(kind, actor, _, _)| (kind, actor))
        .collect();
    assert_eq!(
        kinds,
        vec![
            (EventKind::Added, Actor::User),
            (EventKind::Excepted, Actor::Router),
            (EventKind::Resumed, Actor::Engine),
        ]
    );
    assert_eq!(
        notices(client.window().await),
        vec![ChatNotice::ConstraintResumed {
            rule: RULE.to_owned()
        }]
    );
    let history = flow
        .engine
        .store
        .history_page(flow.chat, None, 50)
        .await
        .unwrap();
    let lines = history
        .entries
        .iter()
        .filter(|entry| matches!(entry, crate::store::HistoryEntry::Constraint { .. }))
        .count();
    assert_eq!(
        lines, 3,
        "the history redraws added, paused and resumed lines"
    );
}

#[tokio::test]
async fn scoped_exception_survives_the_end_of_the_task() {
    let (mut flow, _) = with_constraint(ON, vec![steer_reply(), change_reply(1, "scoped_1")]).await;
    flow.submit("tests/ 폴더에서는 풀어줘").await;

    flow.engine
        .finish_task(flow.chat, flow.agent())
        .await
        .unwrap();

    let stored = constraints(&flow).await;
    assert_eq!(
        stored[0].exception.as_ref().map(|found| found.kind),
        Some(ExceptionKind::Scoped)
    );
    assert_eq!(events(&flow).await.len(), 2);
}

#[tokio::test]
async fn router_change_is_not_asked_or_applied_unless_auto_apply_is_on_even_in_full_mode() {
    for config in [
        "",
        "constraint.auto_apply = false\n",
        "permission.mode = \"full\"\n",
        "constraint.auto_apply = false\npermission.mode = \"full\"\n",
    ] {
        let (mut flow, _) = with_constraint(
            config,
            vec![
                steer_reply(),
                change_reply(1, "release_1"),
                change_distribution_reply(&[0.1, 0.45, 0.45, 0.0]),
            ],
        )
        .await;
        let before = flow.router_calls();

        flow.submit(ASK).await;

        assert_eq!(
            flow.router_calls() - before,
            1,
            "{config:?}: only the route call"
        );
        let stored = constraints(&flow).await;
        assert_eq!(stored[0].state, ConstraintState::Active, "{config:?}");
        assert_eq!(stored[0].exception, None, "{config:?}");
        assert_eq!(events(&flow).await.len(), 1, "{config:?}");
    }
}

#[tokio::test]
async fn unsure_kind_pauses_only_in_full_mode_and_otherwise_changes_nothing() {
    let unsure = || change_distribution_reply(&[0.1, 0.45, 0.45, 0.0]);
    let (mut flow, _) = with_constraint(ON, vec![steer_reply(), unsure()]).await;
    flow.submit(ASK).await;
    assert_eq!(
        events(&flow).await.len(),
        1,
        "no ask window yet, nothing changes"
    );
    assert_eq!(constraints(&flow).await[0].exception, None);

    let full = format!("{ON}permission.mode = \"full\"\n");
    let (mut flow, _) = with_constraint(&full, vec![steer_reply(), unsure()]).await;
    let mut client = flow.client().await;
    flow.submit(ASK).await;

    let stored = constraints(&flow).await;
    assert_eq!(
        stored[0].state,
        ConstraintState::Active,
        "never a permanent release"
    );
    assert_eq!(
        stored[0].exception.as_ref().map(|found| found.kind),
        Some(ExceptionKind::Once)
    );
    let recorded = events(&flow).await;
    assert_eq!(recorded[1].2, Some(EventReason::Unconfirmed));
    assert_eq!(
        notices(client.window().await),
        vec![ChatNotice::ConstraintPaused {
            rule: RULE.to_owned(),
            unconfirmed: true
        }]
    );
}

#[tokio::test]
async fn failed_or_invalid_change_answers_change_nothing() {
    let invalid = change_distribution_reply(&[0.1, 0.9, 0.9, 0.9]);
    for reply in [super::key_rejected(), invalid] {
        let (mut flow, _) = with_constraint(ON, vec![steer_reply(), reply]).await;

        let second = flow.submit(ASK).await;

        assert_eq!(constraints(&flow).await[0].state, ConstraintState::Active);
        assert_eq!(events(&flow).await.len(), 1);
        assert_ne!(
            flow.state(second),
            saturn_protocol::state::InputState::Cancelled
        );
    }
}

#[tokio::test]
async fn task_exception_is_skipped_when_the_input_has_no_task_yet() {
    let (mut flow, _) = with_constraint(
        ON,
        vec![
            running_constraint_reply(0.95, "continues", "queue", 0.1),
            change_reply(1, "once_1"),
        ],
    )
    .await;

    let second = flow.submit(ASK).await;

    assert_eq!(flow.record(second).task, None);
    assert_eq!(constraints(&flow).await[0].exception, None);
    assert_eq!(events(&flow).await.len(), 1);
}

#[tokio::test]
async fn change_judged_against_an_older_constraint_revision_is_judged_again_once() {
    let (mut flow, id) = with_constraint(
        ON,
        vec![
            steer_reply(),
            change_reply(1, "release_1"),
            change_reply(1, "none"),
        ],
    )
    .await;
    let before = flow.router_calls();
    // 라우팅 호출을 먼저 받고 변경 호출만 붙잡는다
    flow.engine
        .submit_input(CLIENT, flow.chat, 1, ASK.to_owned(), false)
        .await
        .unwrap();
    let route = flow.engine.flow.router_rx.recv().await.unwrap();
    let held = flow.transport.hold_next_call();
    flow.engine.on_routed(route).await;
    assert!(flow.is_changing());
    let first_input = constraints(&flow).await[0].input;
    // 판단이 도는 사이 제약 상태가 바뀌었다
    let revision = flow
        .engine
        .store
        .constraint_revision(flow.chat)
        .await
        .unwrap();
    let other = flow
        .engine
        .store
        .register_constraints(&NewRegistration {
            chat: flow.chat,
            input: first_input,
            rules: &[NewRule {
                line: 0,
                rule: "다른 규칙".to_owned(),
                scope: Vec::new(),
            }],
            state: ConstraintState::Active,
            actor: Actor::User,
            reason: None,
            judgment: None,
        })
        .await
        .unwrap();
    assert!(!other.constraints.is_empty());
    assert_ne!(
        flow.engine
            .store
            .constraint_revision(flow.chat)
            .await
            .unwrap(),
        revision
    );
    held.notify_one();
    flow.settle().await;

    let stored = constraints(&flow).await;
    assert_eq!(
        stored
            .iter()
            .find(|constraint| constraint.id == id)
            .unwrap()
            .state,
        ConstraintState::Active,
        "the old judgment must not overwrite the newer state"
    );
    assert_eq!(
        flow.router_calls() - before,
        3,
        "route, the held change call and one re-judgment"
    );
}

#[tokio::test]
async fn change_of_a_canceled_input_is_not_applied() {
    let (mut flow, _) = with_constraint(
        ON,
        vec![
            running_constraint_reply(0.95, "continues", "queue", 0.1),
            change_reply(1, "release_1"),
        ],
    )
    .await;
    flow.engine
        .submit_input(CLIENT, flow.chat, 1, ASK.to_owned(), false)
        .await
        .unwrap();
    let route = flow.engine.flow.router_rx.recv().await.unwrap();
    let input = route.job.input;
    let held = flow.transport.hold_next_call();
    flow.engine.on_routed(route).await;
    assert!(flow.is_changing());

    flow.engine.cancel_input(CLIENT, input).await.unwrap();
    held.notify_one();
    flow.settle().await;

    assert_eq!(constraints(&flow).await[0].state, ConstraintState::Active);
    assert_eq!(events(&flow).await.len(), 1);
}

#[tokio::test]
async fn explicit_release_is_the_user_and_ignores_auto_apply_and_full_mode() {
    let (mut flow, id) = with_constraint("permission.mode = \"full\"\n", Vec::new()).await;
    let mut client = flow.client().await;
    let revision = flow
        .engine
        .store
        .constraint_revision(flow.chat)
        .await
        .unwrap();

    flow.engine.release_constraint(id, revision).await.unwrap();

    assert_eq!(constraints(&flow).await[0].state, ConstraintState::Released);
    let recorded = events(&flow).await;
    assert_eq!(
        (recorded[1].0, recorded[1].1, recorded[1].3),
        (EventKind::Released, Actor::User, false)
    );
    assert_eq!(
        notices(client.window().await),
        vec![ChatNotice::ConstraintReleased {
            rule: RULE.to_owned()
        }]
    );
    // 이미 해제된 제약과 낡은 revision은 거절하고 아무것도 바꾸지 않는다
    let again = flow.engine.release_constraint(id, revision + 1).await;
    assert!(matches!(again, Err(EngineError::StaleConstraint)));
    assert_eq!(events(&flow).await.len(), 2);
}

#[tokio::test]
async fn explicit_release_with_a_stale_revision_changes_nothing() {
    let (mut flow, id) = with_constraint("", Vec::new()).await;

    let result = flow.engine.release_constraint(id, 0).await;

    assert!(matches!(result, Err(EngineError::StaleConstraint)));
    assert_eq!(constraints(&flow).await[0].state, ConstraintState::Active);
    assert_eq!(events(&flow).await.len(), 1);
}

#[tokio::test]
async fn task_exception_of_a_vanished_task_is_closed_at_startup() {
    let (flow, id) = with_constraint(ON, Vec::new()).await;
    let revision = flow
        .engine
        .store
        .constraint_revision(flow.chat)
        .await
        .unwrap();
    let applied = flow
        .engine
        .store
        .change_constraint(&NewChange {
            chat: flow.chat,
            constraint: id,
            change: ConstraintChange::Once { task: TaskId(404) },
            actor: Actor::Router,
            reason: None,
            input: None,
            judgment: None,
            revision,
        })
        .await
        .unwrap();
    assert!(matches!(applied, ChangeOutcome::Applied { .. }));
    let Flow {
        engine,
        fixture,
        chat,
        ..
    } = flow;
    drop(engine);

    let restarted = Restarted::start_with_replies(fixture, chat, Vec::new()).await;

    let stored = restarted
        .engine
        .store
        .constraints_of_chat(chat)
        .await
        .unwrap();
    assert_eq!(stored[0].state, ConstraintState::Active);
    assert_eq!(stored[0].exception, None);
    let kinds: Vec<EventKind> = restarted
        .engine
        .store
        .constraint_events_of_chat(chat)
        .await
        .unwrap()
        .into_iter()
        .map(|(kind, ..)| kind)
        .collect();
    assert_eq!(
        kinds,
        vec![EventKind::Added, EventKind::Excepted, EventKind::Resumed]
    );
}
