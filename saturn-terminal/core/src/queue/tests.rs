use super::*;

const CHAT: ChatId = ChatId(1);

fn scope(paths: &[&str]) -> Vec<PathBuf> {
    paths.iter().map(PathBuf::from).collect()
}

fn input(id: u64, permission: Permission) -> QueuedInput {
    QueuedInput {
        id: InputId(id),
        chat: CHAT,
        text: format!("input {id}"),
        settings: SettingsRevision(1),
        permission,
        workdir: PathBuf::from("/work"),
        write_scope: vec![PathBuf::from("/work")],
        pinned_model: None,
        skip_relation: false,
        state: InputState::Queued,
        reason: None,
        task: None,
    }
}

fn decision(revision: ChatRevision, disposition: Disposition) -> RouteDecision {
    RouteDecision {
        revision,
        settings: SettingsRevision(1),
        disposition,
        is_conflict: false,
        keep_current: true,
        model: None,
        resume_held: false,
        fallbacks: Vec::new(),
    }
}

fn state_of(queue: &Queue, id: u64) -> InputState {
    queue.input(InputId(id)).expect("input should exist").state
}

fn accept_routed(queue: &mut Queue, id: u64, permission: Permission, disposition: Disposition) {
    queue.accept(input(id, permission));
    let revision = queue.revision(CHAT);
    queue
        .apply(InputId(id), &decision(revision, disposition), revision)
        .expect("apply should succeed");
}

/// 관계 판단이 `conflicts`라 끼워 넣기로 정한 입력.
fn accept_conflict(queue: &mut Queue, id: u64, permission: Permission) {
    queue.accept(input(id, permission));
    let revision = queue.revision(CHAT);
    let conflict = RouteDecision {
        is_conflict: true,
        ..decision(revision, Disposition::Steer)
    };
    queue
        .apply(InputId(id), &conflict, revision)
        .expect("apply should succeed");
}

/// 끼워 넣기로 내주기만 하고 전송 중으로 바꾸지 않는다(provider가 끼워 넣기를 지원하지 않는 경우).
fn deliver_steer_without_delivering(queue: &mut Queue) -> InputId {
    let Some(SendAction::Steer { input, .. }) = queue.next_to_send() else {
        panic!("input should steer");
    };
    input
}

/// 끼워 넣기로 내준 입력을 전송 중으로 바꾼다.
fn deliver_steer(queue: &mut Queue) -> InputId {
    let Some(SendAction::Steer { input, .. }) = queue.next_to_send() else {
        panic!("input should steer");
    };
    queue.set_state(input, InputState::Delivering).unwrap();
    input
}

fn start_running(queue: &mut Queue, id: u64, permission: Permission, agent: u64) -> TaskId {
    accept_routed(queue, id, permission, Disposition::NewTask);
    let Some(SendAction::NewTask { task, .. }) = queue.next_to_send() else {
        panic!("input should start a new task");
    };
    queue
        .set_state(InputId(id), InputState::Delivering)
        .unwrap();
    queue.start_task(task, AgentId(agent)).unwrap();
    task
}

#[test]
fn accept_sets_judging_and_later_inputs_wait_for_router_order() {
    let mut queue = Queue::new();

    queue.accept(input(1, Permission::ReadOnly));
    queue.accept(input(2, Permission::ReadOnly));

    assert_eq!(state_of(&queue, 1), InputState::Judging);
    assert_eq!(queue.input(InputId(1)).unwrap().reason, None);
    assert_eq!(
        queue.input(InputId(2)).unwrap().reason,
        Some(QueueReason::RouterOrder)
    );
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 테스트 입력 수
// basis: estimate
#[test]
fn next_to_route_follows_ack_order_one_at_a_time() {
    let mut queue = Queue::new();
    queue.accept(input(1, Permission::ReadOnly));
    queue.accept(input(2, Permission::ReadOnly));

    let first = queue.next_to_route(CHAT).map(|(id, _)| id);
    let revision = queue.revision(CHAT);
    queue
        .apply(
            InputId(1),
            &decision(revision, Disposition::Queue),
            revision,
        )
        .unwrap();
    let second = queue.next_to_route(CHAT).map(|(id, _)| id);

    assert_eq!(first, Some(InputId(1)));
    assert_eq!(second, Some(InputId(2)));
}

#[test]
fn next_to_route_other_chat_returns_none() {
    let mut queue = Queue::new();
    queue.accept(input(1, Permission::ReadOnly));

    assert_eq!(queue.next_to_route(ChatId(9)), None);
}

#[test]
fn apply_changed_revision_returns_conflict() {
    let mut queue = Queue::new();
    queue.accept(input(1, Permission::ReadOnly));
    let (_, routed_at) = queue.next_to_route(CHAT).unwrap();
    start_running(&mut queue, 2, Permission::ReadOnly, 7);

    let result = queue.apply(
        InputId(1),
        &decision(routed_at, Disposition::Steer),
        queue.revision(CHAT),
    );

    assert!(matches!(result, Err(QueueError::RevisionConflict)));
    assert_eq!(state_of(&queue, 1), InputState::Judging);
}

#[test]
fn apply_second_conflict_can_fall_back_to_queue() {
    let mut queue = Queue::new();
    queue.accept(input(1, Permission::ReadOnly));
    let stale = decision(ChatRevision(99), Disposition::Steer);
    let current = queue.revision(CHAT);
    assert!(queue.apply(InputId(1), &stale, current).is_err());
    assert!(queue.apply(InputId(1), &stale, current).is_err());

    queue.set_state(InputId(1), InputState::Queued).unwrap();

    assert_eq!(state_of(&queue, 1), InputState::Queued);
}

#[test]
fn apply_unknown_input_returns_not_found() {
    let mut queue = Queue::new();

    let result = queue.apply(
        InputId(5),
        &decision(ChatRevision(0), Disposition::Queue),
        ChatRevision(0),
    );

    assert!(matches!(result, Err(QueueError::NotFound(InputId(5)))));
}

#[test]
fn apply_bumps_revision() {
    let mut queue = Queue::new();
    let before = queue.revision(CHAT);

    accept_routed(&mut queue, 1, Permission::ReadOnly, Disposition::Queue);

    assert!(queue.revision(CHAT) > before);
}

#[test]
fn next_to_send_first_input_starts_main_task() {
    let mut queue = Queue::new();
    accept_routed(&mut queue, 1, Permission::Write, Disposition::Queue);

    let action = queue.next_to_send();

    assert_eq!(
        action,
        Some(SendAction::NewTask {
            input: InputId(1),
            task: TaskId(1)
        })
    );
}

#[test]
fn next_to_send_does_not_dispatch_twice() {
    let mut queue = Queue::new();
    accept_routed(&mut queue, 1, Permission::ReadOnly, Disposition::Queue);

    let first = queue.next_to_send();
    let second = queue.next_to_send();

    assert!(first.is_some());
    assert_eq!(second, None);
}

#[test]
fn next_to_send_waits_for_dispatched_input_in_same_chat() {
    let mut queue = Queue::new();
    accept_routed(&mut queue, 1, Permission::ReadOnly, Disposition::NewTask);
    accept_routed(&mut queue, 2, Permission::ReadOnly, Disposition::NewTask);

    assert!(queue.next_to_send().is_some());

    assert_eq!(queue.next_to_send(), None);
}

#[test]
fn next_to_send_except_skips_busy_chats_but_not_others() {
    let other = ChatId(2);
    let mut queue = Queue::new();
    accept_routed(&mut queue, 1, Permission::ReadOnly, Disposition::Queue);
    let mut second = input(2, Permission::ReadOnly);
    second.chat = other;
    queue.accept(second);
    let revision = queue.revision(other);
    queue
        .apply(
            InputId(2),
            &decision(revision, Disposition::Queue),
            revision,
        )
        .expect("apply should succeed");

    let sent = queue.next_to_send_except(&[CHAT]);

    assert!(matches!(sent, Some(SendAction::NewTask { input, .. }) if input == InputId(2)));
    assert!(queue.next_to_send_except(&[CHAT, other]).is_none());
    assert!(matches!(
        queue.next_to_send(),
        Some(SendAction::NewTask { input, .. }) if input == InputId(1)
    ));
}

#[test]
fn next_to_send_steer_goes_to_running_agent() {
    let mut queue = Queue::new();
    start_running(&mut queue, 1, Permission::Write, 7);
    accept_routed(&mut queue, 2, Permission::Write, Disposition::Steer);

    let action = queue.next_to_send();

    assert_eq!(
        action,
        Some(SendAction::Steer {
            input: InputId(2),
            agent: AgentId(7)
        })
    );
}

#[test]
fn next_to_send_queue_waits_for_main_then_new_turn() {
    let mut queue = Queue::new();
    start_running(&mut queue, 1, Permission::Write, 7);
    accept_routed(&mut queue, 2, Permission::Write, Disposition::Queue);
    assert_eq!(queue.next_to_send(), None);

    queue.finish_task(AgentId(7));
    let action = queue.next_to_send();

    assert_eq!(
        action,
        Some(SendAction::NewTurn {
            input: InputId(2),
            agent: AgentId(7)
        })
    );
}

#[test]
fn next_to_send_second_writer_waits_for_tree_idle() {
    let mut queue = Queue::new();
    start_running(&mut queue, 1, Permission::Write, 7);
    accept_routed(&mut queue, 2, Permission::Write, Disposition::NewTask);

    let blocked = queue.next_to_send();
    let reason = queue.input(InputId(2)).unwrap().reason;
    queue.finish_task(AgentId(7));
    let released = queue.next_to_send();

    assert_eq!(blocked, None);
    assert_eq!(reason, Some(QueueReason::WriteTurn));
    assert_eq!(
        released,
        Some(SendAction::NewTask {
            input: InputId(2),
            task: TaskId(2)
        })
    );
}

#[test]
fn next_to_send_pending_writer_blocks_other_writer() {
    let other = ChatId(2);
    let mut queue = Queue::new();
    let mut first = input(1, Permission::Write);
    first.chat = other;
    queue.accept(first);
    let revision = queue.revision(other);
    queue
        .apply(
            InputId(1),
            &decision(revision, Disposition::NewTask),
            revision,
        )
        .unwrap();
    accept_routed(&mut queue, 2, Permission::Write, Disposition::NewTask);
    let Some(SendAction::NewTask { task, .. }) = queue.next_to_send() else {
        panic!("first writer should start a new task");
    };

    let blocked = queue.next_to_send();
    let reason = queue.input(InputId(2)).unwrap().reason;
    queue.set_state(InputId(1), InputState::Delivering).unwrap();
    queue.start_task(task, AgentId(7)).unwrap();
    queue.finish_task(AgentId(7));
    let released = queue.next_to_send();

    assert_eq!(blocked, None);
    assert_eq!(reason, Some(QueueReason::WriteTurn));
    assert_eq!(
        released,
        Some(SendAction::NewTask {
            input: InputId(2),
            task: TaskId(2)
        })
    );
}

#[test]
fn next_to_send_read_only_runs_beside_writer() {
    let mut queue = Queue::new();
    start_running(&mut queue, 1, Permission::Write, 7);
    accept_routed(&mut queue, 2, Permission::ReadOnly, Disposition::NewTask);

    let action = queue.next_to_send();

    assert_eq!(
        action,
        Some(SendAction::NewTask {
            input: InputId(2),
            task: TaskId(2)
        })
    );
}

#[test]
fn next_to_send_keeps_order_within_chat() {
    let mut queue = Queue::new();
    start_running(&mut queue, 1, Permission::Write, 7);
    accept_routed(&mut queue, 2, Permission::Write, Disposition::Queue);
    accept_routed(&mut queue, 3, Permission::ReadOnly, Disposition::NewTask);

    let action = queue.next_to_send();

    assert_eq!(action, None);
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 테스트 입력 수
// basis: estimate
#[test]
fn finish_task_closes_auxiliary_agent() {
    let mut queue = Queue::new();
    start_running(&mut queue, 1, Permission::ReadOnly, 7);
    let aux = start_running(&mut queue, 2, Permission::ReadOnly, 8);

    queue.finish_task(AgentId(8));

    let slot = queue.tasks.iter().find(|slot| slot.id == aux).unwrap();
    assert_eq!(slot.phase, TaskPhase::Closed);
}

#[test]
fn defer_steer_turns_steer_into_queue() {
    let mut queue = Queue::new();
    start_running(&mut queue, 1, Permission::Write, 7);
    accept_routed(&mut queue, 2, Permission::Write, Disposition::Steer);
    assert!(matches!(
        queue.next_to_send(),
        Some(SendAction::Steer { .. })
    ));

    queue.defer_steer(InputId(2)).unwrap();

    assert_eq!(queue.disposition(InputId(2)), Some(Disposition::Queue));
    assert_eq!(queue.next_to_send(), None);
    queue.finish_task(AgentId(7));
    assert_eq!(
        queue.next_to_send(),
        Some(SendAction::NewTurn {
            input: InputId(2),
            agent: AgentId(7)
        })
    );
}

#[test]
fn send_now_steers_a_running_task_ahead_of_earlier_waiting_inputs() {
    let mut queue = Queue::new();
    start_running(&mut queue, 1, Permission::Write, 7);
    accept_routed(&mut queue, 2, Permission::Write, Disposition::Queue);
    accept_routed(&mut queue, 3, Permission::Write, Disposition::Queue);

    let previous = queue.send_now(InputId(3)).unwrap();

    assert_eq!(previous, Some(Disposition::Queue));
    assert_eq!(
        queue.inputs_in_state(CHAT, InputState::Queued),
        vec![InputId(3), InputId(2)]
    );
    assert_eq!(
        queue.next_to_send(),
        Some(SendAction::Steer {
            input: InputId(3),
            agent: AgentId(7)
        })
    );
}

#[test]
fn send_now_that_cannot_steer_waits_first_in_line() {
    let mut queue = Queue::new();
    start_running(&mut queue, 1, Permission::Write, 7);
    accept_routed(&mut queue, 2, Permission::Write, Disposition::Queue);
    accept_routed(&mut queue, 3, Permission::Write, Disposition::Queue);
    queue.send_now(InputId(3)).unwrap();
    let Some(SendAction::Steer { input, .. }) = queue.next_to_send() else {
        panic!("input should steer");
    };
    queue.defer_steer(input).unwrap();

    queue.finish_task(AgentId(7));

    assert!(matches!(
        queue.next_to_send(),
        Some(SendAction::NewTurn {
            input: InputId(3),
            ..
        })
    ));
}

#[test]
fn send_now_refuses_inputs_that_are_not_waiting() {
    let mut queue = Queue::new();
    queue.accept(input(1, Permission::Write));
    accept_routed(&mut queue, 2, Permission::Write, Disposition::NewTask);
    queue.next_to_send();

    assert!(matches!(
        queue.send_now(InputId(1)),
        Err(QueueError::InvalidTransition { .. })
    ));
    assert!(matches!(
        queue.send_now(InputId(2)),
        Err(QueueError::AlreadySent)
    ));
    assert!(matches!(
        queue.send_now(InputId(9)),
        Err(QueueError::NotFound(InputId(9)))
    ));
}

#[test]
fn refused_steer_returns_to_the_front_as_a_queued_input() {
    let mut queue = Queue::new();
    start_running(&mut queue, 1, Permission::Write, 7);
    accept_routed(&mut queue, 2, Permission::Write, Disposition::Queue);
    accept_routed(&mut queue, 3, Permission::Write, Disposition::Steer);
    let Some(SendAction::Steer { input, .. }) = queue.next_to_send() else {
        panic!("input should steer");
    };
    queue.set_state(input, InputState::Delivering).unwrap();

    queue.return_refused_steer(input).unwrap();

    assert_eq!(state_of(&queue, 3), InputState::Queued);
    assert_eq!(queue.disposition(InputId(3)), Some(Disposition::Queue));
    assert_eq!(
        queue.inputs_in_state(CHAT, InputState::Queued),
        vec![InputId(3), InputId(2)]
    );
    assert_eq!(queue.next_to_send(), None);
    queue.finish_task(AgentId(7));
    assert!(matches!(
        queue.next_to_send(),
        Some(SendAction::NewTurn {
            input: InputId(3),
            ..
        })
    ));
}

// 근거: #36 결정 (충돌 입력만 끼워 넣기 거절 때 사용자에게 묻고, 그 밖의 입력은 #60 규칙 그대로)
#[test]
fn refused_steer_asks_the_user_only_for_a_conflict_input() {
    let mut queue = Queue::new();
    start_running(&mut queue, 1, Permission::Write, 7);
    accept_conflict(&mut queue, 2, Permission::Write);
    accept_routed(&mut queue, 3, Permission::Write, Disposition::Steer);
    let conflict = deliver_steer(&mut queue);
    let plain = deliver_steer(&mut queue);

    let asks_for_conflict = queue.return_refused_steer(conflict).unwrap();
    let asks_for_plain = queue.return_refused_steer(plain).unwrap();

    assert!(asks_for_conflict);
    assert!(!asks_for_plain);
    assert!(queue.awaits_stop(conflict));
    assert_eq!(
        queue.input(conflict).unwrap().reason,
        Some(QueueReason::ConfirmStop)
    );
    assert_eq!(queue.input(plain).unwrap().reason, None);
    assert_eq!(queue.next_to_send(), None);
    assert_eq!(
        queue.input(conflict).unwrap().reason,
        Some(QueueReason::ConfirmStop)
    );
}

#[test]
fn deferred_steer_asks_the_user_only_for_a_conflict_input() {
    let mut queue = Queue::new();
    start_running(&mut queue, 1, Permission::Write, 7);
    accept_conflict(&mut queue, 2, Permission::Write);
    accept_routed(&mut queue, 3, Permission::Write, Disposition::Steer);
    let conflict = deliver_steer_without_delivering(&mut queue);
    let plain = deliver_steer_without_delivering(&mut queue);

    assert!(queue.defer_steer(conflict).unwrap());
    assert!(!queue.defer_steer(plain).unwrap());
}

#[test]
fn keep_waiting_ends_the_question_and_the_input_takes_the_next_turn() {
    let mut queue = Queue::new();
    start_running(&mut queue, 1, Permission::Write, 7);
    accept_conflict(&mut queue, 2, Permission::Write);
    let input = deliver_steer(&mut queue);
    queue.return_refused_steer(input).unwrap();

    queue.keep_waiting(input).unwrap();

    assert!(!queue.awaits_stop(input));
    assert_eq!(queue.next_to_send(), None);
    assert_eq!(queue.input(input).unwrap().reason, None);
    queue.finish_task(AgentId(7));
    assert!(matches!(
        queue.next_to_send(),
        Some(SendAction::NewTurn {
            input: InputId(2),
            ..
        })
    ));
}

#[test]
fn stop_ends_the_question_and_a_second_answer_is_refused() {
    let mut queue = Queue::new();
    start_running(&mut queue, 1, Permission::Write, 7);
    accept_conflict(&mut queue, 2, Permission::Write);
    let input = deliver_steer(&mut queue);
    queue.return_refused_steer(input).unwrap();

    queue.stop(CHAT);

    assert!(!queue.awaits_stop(input));
    assert_eq!(state_of(&queue, 2), InputState::Held);
    assert!(matches!(
        queue.keep_waiting(input),
        Err(QueueError::NotAwaitingStop(InputId(2)))
    ));
}

#[test]
fn refused_steer_needs_a_delivering_input() {
    let mut queue = Queue::new();
    accept_routed(&mut queue, 1, Permission::Write, Disposition::Queue);

    assert!(matches!(
        queue.return_refused_steer(InputId(1)),
        Err(QueueError::InvalidTransition { .. })
    ));
}

#[test]
fn set_state_follows_transition_table() {
    let mut queue = Queue::new();
    accept_routed(&mut queue, 1, Permission::ReadOnly, Disposition::Queue);

    queue.set_state(InputId(1), InputState::Delivering).unwrap();
    queue.set_state(InputId(1), InputState::Applied).unwrap();

    assert_eq!(state_of(&queue, 1), InputState::Applied);
}

#[test]
fn set_state_outside_table_returns_error() {
    let mut queue = Queue::new();
    accept_routed(&mut queue, 1, Permission::ReadOnly, Disposition::Queue);
    queue.set_state(InputId(1), InputState::Delivering).unwrap();

    let result = queue.set_state(InputId(1), InputState::Queued);

    assert!(matches!(
        result,
        Err(QueueError::InvalidTransition {
            from: InputState::Delivering,
            to: InputState::Queued
        })
    ));
}

#[test]
fn set_state_unknown_input_returns_not_found() {
    let mut queue = Queue::new();

    let result = queue.set_state(InputId(3), InputState::Queued);

    assert!(matches!(result, Err(QueueError::NotFound(InputId(3)))));
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 테스트 입력 수
// basis: estimate
#[test]
fn can_move_terminal_states_have_no_next() {
    let all = [
        InputState::Judging,
        InputState::Queued,
        InputState::Delivering,
        InputState::Applied,
        InputState::Rejected,
        InputState::Held,
        InputState::Cancelled,
    ];

    for terminal in [
        InputState::Applied,
        InputState::Rejected,
        InputState::Cancelled,
    ] {
        assert!(all.iter().all(|to| !can_move(terminal, *to)));
    }
}

#[test]
fn cancel_queued_input_cancels() {
    let mut queue = Queue::new();
    accept_routed(&mut queue, 1, Permission::ReadOnly, Disposition::Queue);

    queue.cancel(InputId(1)).unwrap();

    assert_eq!(state_of(&queue, 1), InputState::Cancelled);
}

#[test]
fn cancel_delivering_or_applied_returns_already_sent() {
    let mut queue = Queue::new();
    accept_routed(&mut queue, 1, Permission::ReadOnly, Disposition::Queue);
    accept_routed(&mut queue, 2, Permission::ReadOnly, Disposition::NewTask);
    queue.set_state(InputId(1), InputState::Delivering).unwrap();
    queue.set_state(InputId(2), InputState::Delivering).unwrap();
    queue.set_state(InputId(2), InputState::Applied).unwrap();

    let delivering = queue.cancel(InputId(1));
    let applied = queue.cancel(InputId(2));

    assert!(matches!(delivering, Err(QueueError::AlreadySent)));
    assert!(matches!(applied, Err(QueueError::AlreadySent)));
}

#[test]
fn cancel_dispatched_input_returns_already_sent() {
    let mut queue = Queue::new();
    accept_routed(&mut queue, 1, Permission::ReadOnly, Disposition::Queue);
    assert!(queue.next_to_send().is_some());

    let result = queue.cancel(InputId(1));

    assert!(matches!(result, Err(QueueError::AlreadySent)));
}

#[test]
fn stop_holds_running_task_and_unsent_inputs() {
    let mut queue = Queue::new();
    let task = start_running(&mut queue, 1, Permission::Write, 7);
    accept_routed(&mut queue, 2, Permission::Write, Disposition::Queue);
    queue.accept(input(3, Permission::Write));

    let held = queue.stop(CHAT);

    assert_eq!(held, vec![task]);
    assert_eq!(state_of(&queue, 1), InputState::Delivering);
    assert_eq!(state_of(&queue, 2), InputState::Held);
    assert_eq!(state_of(&queue, 3), InputState::Held);
    assert_eq!(queue.input(InputId(2)).unwrap().task, Some(task));
}

#[test]
fn stop_held_task_is_not_sent_without_resume() {
    let mut queue = Queue::new();
    start_running(&mut queue, 1, Permission::Write, 7);
    accept_routed(&mut queue, 2, Permission::Write, Disposition::Queue);
    queue.stop(CHAT);

    queue.finish_task(AgentId(7));

    assert_eq!(queue.next_to_send(), None);
}

#[test]
fn stop_without_task_holds_input_in_own_task() {
    let mut queue = Queue::new();
    queue.accept(input(4, Permission::Write));

    let held = queue.stop(CHAT);

    assert_eq!(held, vec![TaskId(4)]);
}

#[test]
fn resume_all_requeues_in_ack_order_and_returns_interrupted() {
    let mut queue = Queue::new();
    let first = start_running(&mut queue, 1, Permission::ReadOnly, 7);
    let second = start_running(&mut queue, 2, Permission::ReadOnly, 8);
    accept_routed(&mut queue, 3, Permission::ReadOnly, Disposition::Queue);
    queue.stop(CHAT);
    queue.finish_task(AgentId(7));
    queue.finish_task(AgentId(8));

    let resumed = queue.resume(CHAT, None);

    assert_eq!(resumed, vec![first, second]);
    assert_eq!(state_of(&queue, 3), InputState::Queued);
}

#[test]
fn resume_target_only_resumes_that_task() {
    let mut queue = Queue::new();
    let first = start_running(&mut queue, 1, Permission::ReadOnly, 7);
    let second = start_running(&mut queue, 2, Permission::ReadOnly, 8);
    queue.stop(CHAT);

    let resumed = queue.resume(CHAT, Some(second));

    assert_eq!(resumed, vec![second]);
    assert_eq!(queue.held_tasks(CHAT, None), vec![first]);
}

#[test]
fn resume_writers_run_one_at_a_time() {
    let mut queue = Queue::new();
    let main = start_running(&mut queue, 1, Permission::Write, 7);
    accept_routed(&mut queue, 2, Permission::Write, Disposition::NewTask);
    queue.stop(CHAT);
    queue.finish_task(AgentId(7));
    queue.resume(CHAT, None);
    let mut continued = input(3, Permission::Write);
    continued.task = Some(main);
    queue.accept(continued);
    queue.set_state(InputId(3), InputState::Queued).unwrap();

    let first = queue.next_to_send();
    let second = queue.next_to_send();

    assert_eq!(
        first,
        Some(SendAction::NewTask {
            input: InputId(2),
            task: TaskId(2)
        })
    );
    assert_eq!(second, None);
    assert_eq!(
        queue.input(InputId(3)).unwrap().reason,
        Some(QueueReason::WriteTurn)
    );
}

// #519
#[test]
fn resumed_task_input_is_sent_while_a_new_task_waits_for_its_write_lock() {
    let mut queue = Queue::new();
    let main = start_running(&mut queue, 1, Permission::Write, 7);
    accept_routed(&mut queue, 2, Permission::Write, Disposition::NewTask);
    queue.stop(CHAT);
    queue.resume(CHAT, None);
    let mut confirmation = input(3, Permission::Write);
    confirmation.task = Some(main);
    queue.accept(confirmation);
    queue.set_state(InputId(3), InputState::Queued).unwrap();

    let first = queue.next_to_send();
    let second = queue.next_to_send();

    assert_eq!(
        first,
        Some(SendAction::NewTurn {
            input: InputId(3),
            agent: AgentId(7)
        })
    );
    assert_eq!(second, None);
    assert_eq!(
        queue.input(InputId(2)).unwrap().reason,
        Some(QueueReason::WriteTurn)
    );
}

// #519
#[test]
fn new_task_still_waits_for_the_lock_when_another_task_is_the_holder() {
    let mut queue = Queue::new();
    start_running(&mut queue, 1, Permission::Write, 7);
    accept_routed(&mut queue, 2, Permission::Write, Disposition::NewTask);
    accept_routed(&mut queue, 3, Permission::ReadOnly, Disposition::Queue);

    let sent = queue.next_to_send();

    assert_ne!(
        sent,
        Some(SendAction::NewTask {
            input: InputId(2),
            task: TaskId(2)
        })
    );
    assert_eq!(
        queue.input(InputId(2)).unwrap().reason,
        Some(QueueReason::WriteTurn)
    );
}

#[test]
fn stop_input_bound_to_closed_task_holds_in_new_task() {
    let mut queue = Queue::new();
    start_running(&mut queue, 1, Permission::ReadOnly, 7);
    let aux = start_running(&mut queue, 2, Permission::ReadOnly, 8);
    queue.finish_task(AgentId(8));
    let mut late = input(3, Permission::ReadOnly);
    late.task = Some(aux);
    queue.accept(late);
    queue.stop(CHAT);

    queue.resume(CHAT, None);

    assert_eq!(state_of(&queue, 3), InputState::Queued);
}

#[test]
fn note_resume_signal_closes_after_limit_without_resume() {
    let mut queue = Queue::new();
    let task = start_running(&mut queue, 1, Permission::ReadOnly, 7);
    queue.stop(CHAT);

    let first = queue.note_resume_signal(CHAT, false);
    let second = queue.note_resume_signal(CHAT, false);
    let third = queue.note_resume_signal(CHAT, false);

    assert_eq!(first, None);
    assert_eq!(second, None);
    assert_eq!(third, Some(vec![task]));
}

#[test]
fn note_resume_signal_resume_resets_count() {
    let mut queue = Queue::new();
    start_running(&mut queue, 1, Permission::ReadOnly, 7);
    queue.stop(CHAT);
    queue.note_resume_signal(CHAT, false);
    queue.note_resume_signal(CHAT, false);

    queue.note_resume_signal(CHAT, true);
    let after = queue.note_resume_signal(CHAT, false);

    assert_eq!(after, None);
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 테스트 입력 수
// basis: estimate
#[test]
fn note_resume_signal_without_held_returns_none() {
    let mut queue = Queue::new();

    let result = (0..HELD_IGNORE_LIMIT)
        .map(|_| queue.note_resume_signal(CHAT, false))
        .last()
        .flatten();

    assert_eq!(result, None);
}

#[test]
fn close_held_cancels_unsent_inputs() {
    let mut queue = Queue::new();
    let task = start_running(&mut queue, 1, Permission::Write, 7);
    accept_routed(&mut queue, 2, Permission::Write, Disposition::Queue);
    queue.stop(CHAT);

    let cancelled = queue.close_held(task);

    assert_eq!(cancelled, vec![InputId(2)]);
    assert_eq!(state_of(&queue, 1), InputState::Delivering);
    assert!(queue.held_tasks(CHAT, None).is_empty());
}

#[test]
fn close_held_releases_write_gate() {
    let mut queue = Queue::new();
    let task = start_running(&mut queue, 1, Permission::Write, 7);
    queue.stop(CHAT);

    queue.close_held(task);

    assert!(
        queue
            .write_gate()
            .try_acquire(&scope(&["/work"]), AgentId(8))
    );
}

#[test]
fn start_task_unknown_task_returns_error() {
    let mut queue = Queue::new();

    let result = queue.start_task(TaskId(1), AgentId(1));

    assert!(matches!(result, Err(QueueError::TaskNotFound(TaskId(1)))));
}

#[test]
fn start_task_write_conflict_keeps_task_pending() {
    let mut queue = Queue::new();
    accept_routed(&mut queue, 1, Permission::Write, Disposition::NewTask);
    assert!(queue.next_to_send().is_some());
    queue
        .write_gate()
        .try_acquire(&scope(&["/work"]), AgentId(9));

    let result = queue.start_task(TaskId(1), AgentId(1));

    assert!(matches!(result, Err(QueueError::WriteConflict(TaskId(1)))));
    assert_eq!(queue.tasks[0].phase, TaskPhase::Pending);
    assert_eq!(queue.tasks[0].agent, None);
}

// #327
#[test]
fn try_acquire_follows_the_write_scope_overlap() {
    // (사례, 먼저 잡은 범위, [(요청 범위, 요청 에이전트, 잡히는지)])
    type Attempt<'a> = (&'a [&'a str], u64, bool);
    let cases: [(&str, &[&str], &[Attempt]); 3] = [
        (
            "same workdir",
            &["/work"],
            &[
                (&["/work"], 1, true),
                (&["/work"], 2, false),
                (&["/other"], 2, true),
            ],
        ),
        (
            "overlapping scopes",
            &["/repo"],
            &[
                (&["/repo/sub"], 2, false),
                (&["/other", "/repo/sub/deep"], 2, false),
                (&["/repo-extra"], 3, true),
                (&["/elsewhere"], 4, true),
            ],
        ),
        (
            "shared added folder",
            &["/a", "/shared"],
            &[(&["/b", "/shared"], 2, false), (&["/b"], 2, true)],
        ),
    ];

    for (name, held, attempts) in cases {
        let mut gate = WriteGate::default();
        assert!(
            gate.try_acquire(&scope(held), AgentId(1)),
            "{name}: first holder"
        );
        for (paths, agent, expected) in attempts {
            assert_eq!(
                gate.try_acquire(&scope(paths), AgentId(*agent)),
                *expected,
                "{name}: agent {agent} asks for {paths:?}"
            );
        }
    }
}

#[test]
fn abandon_task_unblocks_write_inputs_waiting_on_pending_writer() {
    let mut queue = Queue::new();
    accept_routed(&mut queue, 1, Permission::Write, Disposition::NewTask);
    let Some(SendAction::NewTask { task, .. }) = queue.next_to_send() else {
        panic!("input should start a new task");
    };
    queue.set_state(InputId(1), InputState::Delivering).unwrap();
    accept_routed(&mut queue, 2, Permission::Write, Disposition::NewTask);
    assert_eq!(queue.next_to_send(), None);

    queue.abandon_task(task);

    assert!(matches!(
        queue.next_to_send(),
        Some(SendAction::NewTask { .. })
    ));
}

#[test]
fn abandon_task_keeps_started_task() {
    let mut queue = Queue::new();
    let task = start_running(&mut queue, 1, Permission::ReadOnly, 7);

    queue.abandon_task(task);

    assert!(queue.is_main_task(task));
    assert_eq!(queue.tasks[0].phase, TaskPhase::Running);
}

#[test]
fn is_main_task_is_true_only_for_first_open_task() {
    let mut queue = Queue::new();
    let first = start_running(&mut queue, 1, Permission::ReadOnly, 7);
    let second = start_running(&mut queue, 2, Permission::ReadOnly, 8);

    assert!(queue.is_main_task(first));
    assert!(!queue.is_main_task(second));
    assert!(!queue.is_main_task(TaskId(99)));
}

#[test]
fn redirect_changes_queued_input_disposition() {
    let mut queue = Queue::new();
    start_running(&mut queue, 1, Permission::ReadOnly, 7);
    accept_routed(&mut queue, 2, Permission::ReadOnly, Disposition::Queue);
    assert_eq!(queue.next_to_send(), None);

    queue.redirect(InputId(2), Disposition::Steer).unwrap();

    assert_eq!(queue.disposition(InputId(2)), Some(Disposition::Steer));
    assert_eq!(
        queue.next_to_send(),
        Some(SendAction::Steer {
            input: InputId(2),
            agent: AgentId(7)
        })
    );
}

#[test]
fn redirect_new_task_starts_separate_task_even_while_running() {
    let mut queue = Queue::new();
    start_running(&mut queue, 1, Permission::ReadOnly, 7);
    accept_routed(&mut queue, 2, Permission::ReadOnly, Disposition::Queue);

    queue.redirect(InputId(2), Disposition::NewTask).unwrap();

    assert_eq!(
        queue.next_to_send(),
        Some(SendAction::NewTask {
            input: InputId(2),
            task: TaskId(2)
        })
    );
}

#[test]
fn redirect_rejects_judging_dispatched_and_unknown_inputs() {
    let mut queue = Queue::new();
    queue.accept(input(1, Permission::ReadOnly));
    accept_routed(&mut queue, 2, Permission::ReadOnly, Disposition::NewTask);
    queue.next_to_send().unwrap();

    assert!(matches!(
        queue.redirect(InputId(1), Disposition::Steer),
        Err(QueueError::InvalidTransition { .. })
    ));
    assert!(matches!(
        queue.redirect(InputId(2), Disposition::Steer),
        Err(QueueError::AlreadySent)
    ));
    assert!(matches!(
        queue.redirect(InputId(9), Disposition::Steer),
        Err(QueueError::NotFound(InputId(9)))
    ));
}

#[test]
fn hold_unsent_returns_a_dispatched_new_task_to_held_and_resume_sends_it_again() {
    let mut queue = Queue::new();
    accept_routed(&mut queue, 1, Permission::Write, Disposition::NewTask);
    let Some(SendAction::NewTask { task, .. }) = queue.next_to_send() else {
        panic!("input should start a new task");
    };

    queue.hold_unsent(InputId(1)).unwrap();

    assert_eq!(state_of(&queue, 1), InputState::Held);
    assert_eq!(queue.next_to_send(), None);
    assert_eq!(queue.resume(CHAT, None), Vec::<TaskId>::new());
    assert_eq!(state_of(&queue, 1), InputState::Queued);
    assert_eq!(
        queue.next_to_send(),
        Some(SendAction::NewTask {
            input: InputId(1),
            task
        })
    );
}

#[test]
fn hold_unsent_on_an_idle_task_turn_frees_the_write_lock() {
    let mut queue = Queue::new();
    let task = start_running(&mut queue, 1, Permission::Write, 7);
    queue.finish_task(AgentId(7));
    accept_routed(&mut queue, 2, Permission::Write, Disposition::Queue);
    assert!(matches!(
        queue.next_to_send(),
        Some(SendAction::NewTurn { .. })
    ));

    queue.hold_unsent(InputId(2)).unwrap();

    assert_eq!(state_of(&queue, 2), InputState::Held);
    assert!(
        queue
            .write_gate()
            .try_acquire(&scope(&["/work"]), AgentId(8))
    );
    assert_eq!(queue.resume(CHAT, Some(task)), Vec::<TaskId>::new());
    assert_eq!(state_of(&queue, 2), InputState::Queued);
}

#[test]
fn hold_unsent_holds_a_delivering_input_the_provider_refused() {
    let mut queue = Queue::new();
    accept_routed(&mut queue, 1, Permission::Write, Disposition::NewTask);
    queue.next_to_send();
    queue.set_state(InputId(1), InputState::Delivering).unwrap();

    queue.hold_unsent(InputId(1)).unwrap();

    assert_eq!(state_of(&queue, 1), InputState::Held);
    assert_eq!(queue.next_to_send(), None);
}

#[test]
fn hold_unsent_rejects_inputs_that_were_not_handed_out() {
    let mut queue = Queue::new();
    queue.accept(input(1, Permission::ReadOnly));
    accept_routed(&mut queue, 2, Permission::ReadOnly, Disposition::Queue);

    assert!(matches!(
        queue.hold_unsent(InputId(1)),
        Err(QueueError::InvalidTransition { .. })
    ));
    assert!(matches!(
        queue.hold_unsent(InputId(9)),
        Err(QueueError::NotFound(InputId(9)))
    ));
}

#[test]
fn has_waiting_and_inputs_in_state_follow_the_chat() {
    let mut queue = Queue::new();
    accept_routed(&mut queue, 1, Permission::Write, Disposition::Queue);
    queue.accept(input(2, Permission::Write));

    assert!(queue.has_waiting(CHAT));
    assert!(!queue.has_waiting(ChatId(9)));
    assert_eq!(
        queue.inputs_in_state(CHAT, InputState::Judging),
        vec![InputId(2)]
    );
    queue.set_state(InputId(1), InputState::Cancelled).unwrap();
    assert!(!queue.has_waiting(CHAT));
}

#[test]
fn release_frees_workdir() {
    let mut gate = WriteGate::default();
    gate.try_acquire(&scope(&["/work"]), AgentId(1));

    gate.release(AgentId(1));

    assert!(gate.try_acquire(&scope(&["/work"]), AgentId(2)));
}

#[test]
fn waiting_inputs_name_the_task_they_wait_for_and_none_for_a_new_task() {
    let mut queue = Queue::new();
    let task = start_running(&mut queue, 1, Permission::ReadOnly, 7);
    accept_routed(&mut queue, 2, Permission::ReadOnly, Disposition::Queue);
    accept_routed(&mut queue, 3, Permission::ReadOnly, Disposition::NewTask);

    let waiting = queue.waiting_inputs();

    assert_eq!(
        waiting,
        [(CHAT, InputId(2), Some(task)), (CHAT, InputId(3), None),]
    );
}

// #455
#[test]
fn rescope_unsent_changes_only_inputs_that_are_not_sent() {
    let mut queue = Queue::new();
    accept_routed(&mut queue, 1, Permission::Write, Disposition::NewTask);
    assert!(queue.next_to_send().is_some());
    queue.accept(input(2, Permission::Write));

    queue.rescope_unsent(CHAT, |_| scope(&["/work", "/shared"]));

    let written = |id: u64| queue.input(InputId(id)).unwrap().write_scope.clone();
    assert_eq!(written(1), scope(&["/work"]));
    assert_eq!(written(2), scope(&["/work", "/shared"]));
}

// #455
#[test]
fn rescope_unsent_ignores_other_chats() {
    let mut queue = Queue::new();
    queue.accept(input(1, Permission::Write));

    queue.rescope_unsent(ChatId(2), |_| scope(&["/shared"]));

    assert_eq!(
        queue.input(InputId(1)).unwrap().write_scope,
        scope(&["/work"])
    );
}

// #488
#[test]
fn rescope_unsent_also_changes_held_inputs() {
    let mut queue = Queue::new();
    queue.accept(input(1, Permission::Write));
    queue.stop(CHAT);
    assert_eq!(state_of(&queue, 1), InputState::Held);

    queue.rescope_unsent(CHAT, |_| scope(&["/work", "/shared"]));

    assert_eq!(
        queue.input(InputId(1)).unwrap().write_scope,
        scope(&["/work", "/shared"])
    );
}

// #506
#[test]
fn pin_model_if_unset_fills_an_empty_pin_and_keeps_an_existing_one() {
    let mut queue = Queue::new();
    queue.accept(input(1, Permission::Write));
    let mut pinned = input(2, Permission::Write);
    pinned.pinned_model = Some("claude/haiku".to_owned());
    queue.accept(pinned);

    queue
        .pin_model_if_unset(InputId(1), "codex/gpt-x")
        .expect("input should exist");
    queue
        .pin_model_if_unset(InputId(2), "codex/gpt-x")
        .expect("input should exist");

    assert_eq!(
        queue.input(InputId(1)).unwrap().pinned_model.as_deref(),
        Some("codex/gpt-x")
    );
    assert_eq!(
        queue.input(InputId(2)).unwrap().pinned_model.as_deref(),
        Some("claude/haiku")
    );
    assert!(queue.pin_model_if_unset(InputId(9), "x").is_err());
}
