//! 제약 관리 테스트: 명시 등록, 목록과 변경 내역, 잘못 등록, 되돌리기를 소켓 요청으로 확인한다. 낡은 revision과
//! 다른 창, 재시작에서 제약이 바뀌지 않는지와 명시 등록 제약이 `auto_apply` 없이 인계 패킷에 들어가는지도 본다.
//! 설계: docs/design/constraints.md#되돌리기

use saturn_protocol::envelope::ServerMessage;
use saturn_protocol::ids::{ConstraintId, SessionId};
use saturn_protocol::rpc::{
    ChatNotice, ConstraintActor, ConstraintChangeInfo, ConstraintChangeKind, ConstraintInfo,
    ConstraintStatus, Notification, QueryResult, Request,
};

use super::constraint_handoff::{packet_of, turn};
use super::crash_recovery::Restarted;
use super::support::{Flow, idle_reply};
use super::*;
use crate::EngineError;
use crate::providers::test_support::{CLAUDE, CODEX};
use crate::store::{Actor, ConstraintState, EventKind, EventReason};

const RULE: &str = "Release builds must never print the internal build token";

/// 요청을 보내고 응답이 올 때까지 engine 요청 처리를 돌린다. 응답 전에 온 알림은 모아 함께 돌려준다.
async fn call(
    flow: &mut Flow,
    client: &mut Client,
    id: u64,
    request: Request,
) -> (Outcome, Vec<Notification>) {
    drive(&mut flow.engine, async {
        client.send(id, request).await;
        let mut seen = Vec::new();
        loop {
            match client.recv().await {
                ServerMessage::Notification(message) => seen.push(message.notification),
                ServerMessage::Response(response) => {
                    assert_eq!(response.id, Some(RequestId(id)));
                    return (response.outcome, seen);
                }
            }
        }
    })
    .await
}

/// 알림 중 제약 줄 하나씩.
fn lines(notifications: &[Notification]) -> Vec<&ChatNotice> {
    notifications
        .iter()
        .filter_map(|notification| match notification {
            Notification::ChatNotice { notice, .. } => Some(notice),
            _ => None,
        })
        .collect()
}

struct Listed {
    revision: u64,
    constraints: Vec<ConstraintInfo>,
    changes: Vec<ConstraintChangeInfo>,
}

async fn list(flow: &mut Flow, client: &mut Client) -> Listed {
    let chat = flow.chat;
    let result = drive(
        &mut flow.engine,
        client.query(99, Request::ListConstraints { chat }),
    )
    .await;
    let QueryResult::Constraints {
        revision,
        constraints,
        changes,
        ..
    } = result
    else {
        panic!("expected the constraint list");
    };
    Listed {
        revision,
        constraints,
        changes,
    }
}

fn is_ok(outcome: &Outcome) -> bool {
    matches!(outcome, Outcome::Ok(_))
}

async fn add(flow: &mut Flow, client: &mut Client, text: &str) -> Outcome {
    let chat = flow.chat;
    call(
        flow,
        client,
        1,
        Request::AddConstraint {
            chat,
            text: text.to_owned(),
        },
    )
    .await
    .0
}

async fn state_of(flow: &Flow, id: ConstraintId) -> ConstraintState {
    flow.engine
        .store
        .constraints_of_chat(flow.chat)
        .await
        .unwrap()
        .into_iter()
        .find(|constraint| constraint.id == id)
        .unwrap()
        .state
}

#[tokio::test]
async fn explicit_add_stores_the_text_as_typed_without_the_router_or_auto_apply() {
    // 설정 없음: constraint.auto_apply는 기본 꺼짐이고 router 답은 하나도 준비하지 않았다
    let mut flow = Flow::with_config("", Vec::new()).await;
    let mut client = flow.client().await;
    let typed = "  에러 메시지는\n  영어로   통일해 ";

    let outcome = {
        let chat = flow.chat;
        call(
            &mut flow,
            &mut client,
            1,
            Request::AddConstraint {
                chat,
                text: typed.to_owned(),
            },
        )
        .await
    };

    assert!(is_ok(&outcome.0), "{:?}", outcome.0);
    assert_eq!(lines(&outcome.1).len(), 1);
    assert_eq!(flow.router_calls(), 0);
    let listed = list(&mut flow, &mut client).await;
    assert_eq!(listed.constraints.len(), 1);
    assert_eq!(listed.constraints[0].rule, typed.trim(), "원문 그대로");
    assert_eq!(listed.constraints[0].status, ConstraintStatus::Active);
    assert_eq!(listed.changes.len(), 1);
    assert_eq!(
        (listed.changes[0].kind, listed.changes[0].actor),
        (ConstraintChangeKind::Added, ConstraintActor::User)
    );
    assert!(listed.changes[0].undoable);
}

#[tokio::test]
async fn explicit_add_rejects_blank_text_and_stores_nothing() {
    let mut flow = Flow::with_config("", Vec::new()).await;
    let mut client = flow.client().await;

    for blank in ["", "   \n\t "] {
        let outcome = add(&mut flow, &mut client, blank).await;
        assert!(matches!(outcome, Outcome::Err(_)), "{blank:?}");
    }

    assert!(list(&mut flow, &mut client).await.constraints.is_empty());
}

#[tokio::test]
async fn explicit_constraint_reaches_the_switch_packet_with_auto_apply_off() {
    let replies = (0..12).map(|_| idle_reply(0.95)).collect();
    let mut flow = Flow::with_config("", replies).await;
    flow.add_provider(CODEX);
    let claude = flow.fake.clone();
    flow.engine.switch_provider(flow.chat, CODEX);
    let mut client = flow.client().await;
    turn(&mut flow, CODEX, "task 1 write the cache", "c1").await;
    assert!(is_ok(&add(&mut flow, &mut client, RULE).await));
    for number in 2..=6 {
        turn(
            &mut flow,
            CODEX,
            &format!("task {number} continue the cache"),
            &format!("c{number}"),
        )
        .await;
    }

    flow.engine.switch_provider(flow.chat, CLAUDE);
    flow.submit("task 7 review the cache").await;

    let packet = packet_of(&claude);
    let (fixed, rest) = packet
        .split_once("## Conversation")
        .expect("conversation section should exist");
    assert!(fixed.contains("## Constraints and decisions"), "{packet}");
    assert!(fixed.contains(RULE), "{packet}");
    assert!(!rest.contains(RULE), "{packet}");
    let rows = flow
        .engine
        .store
        .packet_constraints_of(SessionId(2))
        .await
        .unwrap();
    assert_eq!(rows, vec![(ConstraintId(1), "All".to_owned())]);
}

#[tokio::test]
async fn release_and_mistaken_release_are_user_changes_that_undo_restores() {
    let table = [(false, None), (true, Some(EventReason::Mistaken))];
    for (mistaken, reason) in table {
        let mut flow = Flow::with_config("", Vec::new()).await;
        let mut client = flow.client().await;
        add(&mut flow, &mut client, RULE).await;
        let listed = list(&mut flow, &mut client).await;
        let id = listed.constraints[0].id;

        let (outcome, notified) = call(
            &mut flow,
            &mut client,
            2,
            Request::ReleaseConstraint {
                constraint: id,
                revision: listed.revision,
                mistaken,
            },
        )
        .await;

        assert!(is_ok(&outcome), "{outcome:?}");
        assert!(matches!(
            lines(&notified)[..],
            [ChatNotice::ConstraintReleased { .. }]
        ));
        assert_eq!(state_of(&flow, id).await, ConstraintState::Released);
        let events = flow
            .engine
            .store
            .constraint_events_of_chat(flow.chat)
            .await
            .unwrap();
        assert_eq!(
            (events[1].0, events[1].1, events[1].2),
            (EventKind::Released, Actor::User, reason),
            "mistaken={mistaken}"
        );

        // 해제 줄을 되돌리면 유효로 돌아오고 되돌림 줄이 남는다
        let after = list(&mut flow, &mut client).await;
        let released = after.changes.last().unwrap();
        assert!(released.undoable);
        let (outcome, notified) = call(
            &mut flow,
            &mut client,
            3,
            Request::UndoConstraintChange {
                constraint: id,
                event: released.event,
                revision: after.revision,
            },
        )
        .await;
        assert!(is_ok(&outcome), "{outcome:?}");
        assert!(matches!(
            lines(&notified)[..],
            [ChatNotice::ConstraintRestored { .. }]
        ));
        assert_eq!(state_of(&flow, id).await, ConstraintState::Active);
        let last = list(&mut flow, &mut client).await;
        assert_eq!(
            last.changes.last().unwrap().kind,
            ConstraintChangeKind::Restored
        );
        assert!(!last.changes.last().unwrap().undoable);
        // 되돌린 줄은 더 이상 가장 최근 변경이 아니다
        assert!(
            last.changes
                .iter()
                .all(|change| !change.undoable || change.kind != ConstraintChangeKind::Released)
        );
    }
}

#[tokio::test]
async fn undoing_an_added_constraint_releases_it() {
    let mut flow = Flow::with_config("", Vec::new()).await;
    let mut client = flow.client().await;
    add(&mut flow, &mut client, RULE).await;
    let listed = list(&mut flow, &mut client).await;
    let id = listed.constraints[0].id;

    let (outcome, _) = call(
        &mut flow,
        &mut client,
        2,
        Request::UndoConstraintChange {
            constraint: id,
            event: listed.changes[0].event,
            revision: listed.revision,
        },
    )
    .await;

    assert!(is_ok(&outcome), "{outcome:?}");
    assert_eq!(state_of(&flow, id).await, ConstraintState::Released);
    let after = list(&mut flow, &mut client).await;
    assert_eq!(
        after.changes.last().unwrap().kind,
        ConstraintChangeKind::Restored
    );
}

#[tokio::test]
async fn undoing_an_exception_closes_it_and_keeps_the_constraint_active() {
    let mut flow = Flow::with_config("", Vec::new()).await;
    let mut client = flow.client().await;
    add(&mut flow, &mut client, RULE).await;
    let listed = list(&mut flow, &mut client).await;
    let id = listed.constraints[0].id;
    flow.engine
        .store
        .change_constraint(&crate::store::NewChange {
            chat: flow.chat,
            constraint: id,
            change: crate::store::ConstraintChange::Scoped {
                condition: "only inside tests/".to_owned(),
            },
            actor: Actor::Router,
            reason: None,
            input: None,
            judgment: None,
            revision: listed.revision,
        })
        .await
        .unwrap();
    let listed = list(&mut flow, &mut client).await;
    assert!(listed.constraints[0].exception.is_some());
    let excepted = listed.changes.last().unwrap();
    assert_eq!(excepted.kind, ConstraintChangeKind::Excepted);

    let (outcome, _) = call(
        &mut flow,
        &mut client,
        3,
        Request::UndoConstraintChange {
            constraint: id,
            event: excepted.event,
            revision: listed.revision,
        },
    )
    .await;

    assert!(is_ok(&outcome), "{outcome:?}");
    let after = list(&mut flow, &mut client).await;
    assert_eq!(after.constraints[0].status, ConstraintStatus::Active);
    assert!(after.constraints[0].exception.is_none());
}

#[tokio::test]
async fn stale_or_superseded_requests_change_nothing_across_windows_and_restart() {
    let mut flow = Flow::with_config("", Vec::new()).await;
    let mut first_window = flow.client().await;
    let mut second_window = flow.client().await;
    add(&mut flow, &mut first_window, RULE).await;
    let seen = list(&mut flow, &mut first_window).await;
    let id = seen.constraints[0].id;
    let added = seen.changes[0].event;
    // 두 창이 같은 revision을 봤고 첫 창이 먼저 해제한다
    let second_seen = list(&mut flow, &mut second_window).await;
    let (outcome, _) = call(
        &mut flow,
        &mut first_window,
        2,
        Request::ReleaseConstraint {
            constraint: id,
            revision: seen.revision,
            mistaken: false,
        },
    )
    .await;
    assert!(is_ok(&outcome), "{outcome:?}");

    // 둘째 창의 옛 revision 요청은 해제 뒤 상태를 건드리지 못한다
    second_window.window().await;
    let stale = [
        Request::ReleaseConstraint {
            constraint: id,
            revision: second_seen.revision,
            mistaken: true,
        },
        Request::UndoConstraintChange {
            constraint: id,
            event: added,
            revision: second_seen.revision,
        },
    ];
    for (number, request) in stale.into_iter().enumerate() {
        let (outcome, notified) =
            call(&mut flow, &mut second_window, 10 + number as u64, request).await;
        assert!(matches!(outcome, Outcome::Err(_)), "{outcome:?}");
        assert!(lines(&notified).is_empty());
    }
    // 가장 최근 변경이 아닌 등록 줄은 새 revision으로도 되돌리지 못한다
    let current = list(&mut flow, &mut second_window).await;
    let (outcome, _) = call(
        &mut flow,
        &mut second_window,
        20,
        Request::UndoConstraintChange {
            constraint: id,
            event: added,
            revision: current.revision,
        },
    )
    .await;
    assert!(matches!(outcome, Outcome::Err(_)), "{outcome:?}");
    assert_eq!(state_of(&flow, id).await, ConstraintState::Released);
    let events_before = flow
        .engine
        .store
        .constraint_events_of_chat(flow.chat)
        .await
        .unwrap()
        .len();

    // 재시작해도 같은 revision이고, 옛 revision 요청은 여전히 거절한다
    let mut restarted = Restarted::after_shutdown(flow).await;
    let revision = restarted
        .engine
        .store
        .constraint_revision(restarted.chat)
        .await
        .unwrap();
    assert_eq!(revision, current.revision);
    let result = restarted
        .engine
        .undo_constraint_change(id, added, second_seen.revision)
        .await;
    assert!(matches!(result, Err(EngineError::StaleConstraint)));
    let stored = restarted
        .engine
        .store
        .constraints_of_chat(restarted.chat)
        .await
        .unwrap();
    assert_eq!(stored[0].state, ConstraintState::Released);
    assert_eq!(
        restarted
            .engine
            .store
            .constraint_events_of_chat(restarted.chat)
            .await
            .unwrap()
            .len(),
        events_before
    );
}

#[tokio::test]
async fn list_marks_a_candidate_apart_from_an_active_constraint() {
    let mut flow = Flow::with_config("", vec![idle_reply(0.95)]).await;
    let mut client = flow.client().await;
    add(&mut flow, &mut client, RULE).await;
    let first = flow.submit("first request").await;
    flow.engine
        .store
        .register_constraints(&crate::store::NewRegistration {
            chat: flow.chat,
            input: first,
            rules: &[crate::store::NewRule {
                line: 0,
                rule: "테스트 메시지는 한국어로 둬".to_owned(),
                scope: Vec::new(),
            }],
            state: ConstraintState::Candidate,
            actor: Actor::Router,
            reason: None,
            judgment: None,
        })
        .await
        .unwrap();

    let listed = list(&mut flow, &mut client).await;

    let statuses: Vec<ConstraintStatus> = listed
        .constraints
        .iter()
        .map(|constraint| constraint.status)
        .collect();
    assert_eq!(
        statuses,
        [ConstraintStatus::Active, ConstraintStatus::Candidate]
    );
    // 등록 확인 중인 후보는 변경 내역에 등록 줄이 없다
    assert_eq!(listed.changes.len(), 1);
}
