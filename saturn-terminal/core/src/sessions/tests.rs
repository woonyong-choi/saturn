use super::*;

const CHAT: ChatId = ChatId(1);
const NOW_SECS: u64 = 1_000_000;

fn inputs(packet: u64) -> ReturnInputs {
    ReturnInputs {
        budget: ContextBudget {
            t_abs: 100_000,
            safety_percent: 60,
            window: 200_000,
            cache_read: 0.1,
            cache_write: 1.25,
            cache_ttl: Duration::from_secs(300),
        },
        packet,
        now: SystemTime::UNIX_EPOCH + Duration::from_secs(NOW_SECS),
    }
}

fn last_turn(active: u64, secs_ago: u64) -> LastTurn {
    LastTurn {
        active,
        ended_at: SystemTime::UNIX_EPOCH + Duration::from_secs(NOW_SECS - secs_ago),
    }
}

/// Claude가 열림, Codex 보관(id 1)인 채팅. Codex로 돌아가는 판정을 시험한다.
fn archived_codex(active: u64, secs_ago: u64) -> SessionManager {
    let mut manager = manager_with(vec![
        record(1, Provider::Codex, SessionState::ClosedResumable),
        record(2, Provider::Claude, SessionState::Open),
    ]);
    manager.record_last_turn(SessionId(1), last_turn(active, secs_ago));
    manager
}

fn return_target(manager: &SessionManager, packet: u64) -> SendTarget {
    manager.target_for_send(CHAT, Provider::Codex, AgentRole::Main, &inputs(packet))
}

fn new_codex() -> SendTarget {
    SendTarget::New {
        provider: Provider::Codex,
        role: AgentRole::Main,
    }
}

fn record(id: u64, provider: Provider, state: SessionState) -> SessionRecord {
    SessionRecord {
        id: SessionId(id),
        chat: CHAT,
        agent: AgentId(id),
        role: AgentRole::Main,
        provider,
        provider_session: Some(ProviderSessionId(format!("p{id}"))),
        model: None,
        state,
        delivered: LedgerSeq(0),
        idle_since: None,
    }
}

// cost: time O(s²), heap O(s), stack O(1)
// vars: s = 테스트 session 수
// basis: estimate
fn manager_with(records: Vec<SessionRecord>) -> SessionManager {
    let mut manager = SessionManager::new();
    for record in records {
        manager.register(record).unwrap();
    }
    manager
}

#[test]
fn target_for_send_without_session_returns_new() {
    let manager = SessionManager::new();

    let target = manager.target_for_send(CHAT, Provider::Claude, AgentRole::Main, &inputs(50_000));

    assert_eq!(
        target,
        SendTarget::New {
            provider: Provider::Claude,
            role: AgentRole::Main
        }
    );
}

#[test]
fn target_for_send_open_same_provider_returns_open() {
    let manager = manager_with(vec![record(1, Provider::Claude, SessionState::Open)]);

    let target = manager.target_for_send(CHAT, Provider::Claude, AgentRole::Main, &inputs(50_000));

    assert_eq!(target, SendTarget::Open(SessionId(1)));
}

#[test]
fn target_for_send_closed_session_returns_resume() {
    let manager = manager_with(vec![record(
        1,
        Provider::Codex,
        SessionState::ClosedResumable,
    )]);

    let target = manager.target_for_send(CHAT, Provider::Codex, AgentRole::Main, &inputs(50_000));

    assert_eq!(target, SendTarget::Resume(SessionId(1)));
}

#[test]
fn target_for_send_closed_without_provider_id_returns_new() {
    let mut closed = record(1, Provider::Codex, SessionState::ClosedResumable);
    closed.provider_session = None;
    let manager = manager_with(vec![closed]);

    let target = manager.target_for_send(CHAT, Provider::Codex, AgentRole::Main, &inputs(50_000));

    assert!(matches!(target, SendTarget::New { .. }));
}

#[test]
fn target_for_send_other_provider_returns_new() {
    let manager = manager_with(vec![record(1, Provider::Claude, SessionState::Open)]);

    let target = manager.target_for_send(CHAT, Provider::Codex, AgentRole::Main, &inputs(50_000));

    assert_eq!(
        target,
        SendTarget::New {
            provider: Provider::Codex,
            role: AgentRole::Main
        }
    );
}

#[test]
fn target_for_send_sub_always_returns_new() {
    let manager = manager_with(vec![record(1, Provider::Claude, SessionState::Open)]);

    let target = manager.target_for_send(CHAT, Provider::Claude, AgentRole::Sub, &inputs(50_000));

    assert_eq!(
        target,
        SendTarget::New {
            provider: Provider::Claude,
            role: AgentRole::Sub
        }
    );
}

#[test]
fn register_second_open_main_returns_error() {
    let mut manager = manager_with(vec![record(1, Provider::Claude, SessionState::Open)]);

    let result = manager.register(record(2, Provider::Codex, SessionState::Open));

    assert!(matches!(result, Err(SessionError::MainAlreadyOpen)));
}

#[test]
fn register_duplicate_id_returns_error() {
    let mut manager = manager_with(vec![record(1, Provider::Claude, SessionState::Ended)]);

    let result = manager.register(record(1, Provider::Codex, SessionState::Open));

    assert!(matches!(
        result,
        Err(SessionError::DuplicateId(SessionId(1)))
    ));
}

#[test]
fn register_sub_beside_main_succeeds() {
    let mut manager = manager_with(vec![record(1, Provider::Claude, SessionState::Open)]);
    let mut sub = record(2, Provider::Codex, SessionState::Open);
    sub.role = AgentRole::Sub;

    let result = manager.register(sub);

    assert!(result.is_ok());
}

#[test]
fn replace_during_turn_returns_not_at_turn_boundary() {
    let mut manager = manager_with(vec![record(1, Provider::Claude, SessionState::Open)]);

    let result = manager.replace(SessionId(1), record(2, Provider::Codex, SessionState::Open));

    assert!(matches!(result, Err(SessionError::NotAtTurnBoundary)));
}

// cost: time O(s), heap O(1), stack O(1)
// vars: s = 테스트 session 수
// basis: estimate
#[test]
fn replace_keeps_one_live_session_per_chat() {
    let mut manager = manager_with(vec![record(1, Provider::Claude, SessionState::Open)]);
    manager.mark_idle(SessionId(1), Instant::now());

    manager
        .replace(SessionId(1), record(2, Provider::Codex, SessionState::Open))
        .unwrap();

    let live: Vec<SessionId> = manager
        .sessions
        .iter()
        .filter(|session| session.chat == CHAT && session.state != SessionState::Ended)
        .map(|session| session.id)
        .collect();
    assert_eq!(live, vec![SessionId(2)]);
}

#[test]
fn replace_attaches_only_after_previous_delivered() {
    let mut manager = manager_with(vec![record(1, Provider::Claude, SessionState::Open)]);
    manager.mark_delivered(SessionId(1), LedgerSeq(40));
    manager.mark_idle(SessionId(1), Instant::now());

    manager
        .replace(SessionId(1), record(2, Provider::Codex, SessionState::Open))
        .unwrap();

    assert_eq!(manager.attach_from(SessionId(2)), LedgerSeq(40));
}

#[test]
fn replace_keeps_packet_seq_when_newer() {
    let mut manager = manager_with(vec![record(1, Provider::Claude, SessionState::Held)]);
    manager.mark_delivered(SessionId(1), LedgerSeq(40));
    let mut new = record(2, Provider::Codex, SessionState::Open);
    new.delivered = LedgerSeq(55);

    manager.replace(SessionId(1), new).unwrap();

    assert_eq!(manager.attach_from(SessionId(2)), LedgerSeq(55));
}

#[test]
fn replace_unknown_session_returns_not_found() {
    let mut manager = SessionManager::new();

    let result = manager.replace(SessionId(9), record(2, Provider::Codex, SessionState::Open));

    assert!(matches!(result, Err(SessionError::NotFound(SessionId(9)))));
}

#[test]
fn replace_other_chat_keeps_original_session() {
    let mut manager = manager_with(vec![record(1, Provider::Claude, SessionState::Held)]);
    let mut other_chat = record(2, Provider::Codex, SessionState::Open);
    other_chat.chat = ChatId(2);

    let result = manager.replace(SessionId(1), other_chat);

    assert!(matches!(result, Err(SessionError::InvalidReplacement)));
    assert_eq!(manager.get(SessionId(1)).unwrap().state, SessionState::Held);
}

#[test]
fn replace_duplicate_id_keeps_original_session() {
    let mut manager = manager_with(vec![record(1, Provider::Claude, SessionState::Held)]);

    let result = manager.replace(SessionId(1), record(1, Provider::Codex, SessionState::Open));

    assert!(matches!(
        result,
        Err(SessionError::DuplicateId(SessionId(1)))
    ));
    assert_eq!(manager.get(SessionId(1)).unwrap().state, SessionState::Held);
}

#[test]
fn mark_delivered_never_lowers() {
    let mut manager = manager_with(vec![record(1, Provider::Claude, SessionState::Open)]);
    manager.mark_delivered(SessionId(1), LedgerSeq(10));

    manager.mark_delivered(SessionId(1), LedgerSeq(3));

    assert_eq!(manager.attach_from(SessionId(1)), LedgerSeq(10));
}

#[test]
fn attach_from_unknown_session_returns_start() {
    let manager = SessionManager::new();

    assert_eq!(manager.attach_from(SessionId(1)), LedgerSeq(0));
}

#[test]
fn set_state_follows_transition_table() {
    let mut manager = manager_with(vec![record(1, Provider::Claude, SessionState::Open)]);

    manager.set_state(SessionId(1), SessionState::Held).unwrap();
    manager.set_state(SessionId(1), SessionState::Open).unwrap();
    manager
        .set_state(SessionId(1), SessionState::Ended)
        .unwrap();

    assert_eq!(
        manager.get(SessionId(1)).unwrap().state,
        SessionState::Ended
    );
}

#[test]
fn set_state_from_ended_returns_error() {
    let mut manager = manager_with(vec![record(1, Provider::Claude, SessionState::Ended)]);

    let result = manager.set_state(SessionId(1), SessionState::Open);

    assert!(matches!(
        result,
        Err(SessionError::InvalidTransition {
            from: SessionState::Ended,
            to: SessionState::Open
        })
    ));
}

#[test]
fn set_state_closed_to_held_returns_error() {
    let mut manager = manager_with(vec![record(
        1,
        Provider::Claude,
        SessionState::ClosedResumable,
    )]);

    let result = manager.set_state(SessionId(1), SessionState::Held);

    assert!(result.is_err());
}

#[test]
fn target_for_send_warm_below_threshold_resumes_archive() {
    let manager = archived_codex(99_999, 300);

    assert_eq!(
        return_target(&manager, 50_000),
        SendTarget::Resume(SessionId(1))
    );
}

#[test]
fn target_for_send_warm_above_threshold_resumes_archive() {
    let manager = archived_codex(150_000, 10);

    assert_eq!(
        return_target(&manager, 50_000),
        SendTarget::Resume(SessionId(1))
    );
}

#[test]
fn target_for_send_expired_smaller_packet_returns_new() {
    let manager = archived_codex(80_000, 301);

    assert_eq!(return_target(&manager, 79_999), new_codex());
}

#[test]
fn target_for_send_expired_packet_not_smaller_resumes_archive() {
    let manager = archived_codex(80_000, 301);

    assert_eq!(
        return_target(&manager, 80_000),
        SendTarget::Resume(SessionId(1))
    );
}

#[test]
fn target_for_send_archive_without_last_turn_resumes() {
    let manager = manager_with(vec![
        record(1, Provider::Codex, SessionState::ClosedResumable),
        record(2, Provider::Claude, SessionState::Open),
    ]);

    assert_eq!(
        return_target(&manager, 50_000),
        SendTarget::Resume(SessionId(1))
    );
}

#[test]
fn live_main_prefers_the_open_main_over_a_later_registered_archive() {
    let manager = manager_with(vec![
        record(1, Provider::Codex, SessionState::Open),
        record(2, Provider::Claude, SessionState::ClosedResumable),
    ]);

    assert_eq!(
        manager.live_main(CHAT).map(|main| main.id),
        Some(SessionId(1))
    );
    assert_eq!(
        return_target(&manager, 50_000),
        SendTarget::Open(SessionId(1))
    );
}

#[test]
fn target_for_send_archive_without_provider_id_returns_new() {
    let mut archive = record(1, Provider::Codex, SessionState::ClosedResumable);
    archive.provider_session = None;
    let manager = manager_with(vec![
        archive,
        record(2, Provider::Claude, SessionState::Open),
    ]);

    assert_eq!(return_target(&manager, 50_000), new_codex());
}

#[test]
fn target_for_send_clock_before_last_turn_counts_as_warm() {
    let mut manager = archived_codex(10_000, 0);
    manager.record_last_turn(
        SessionId(1),
        LastTurn {
            active: 150_000,
            ended_at: SystemTime::UNIX_EPOCH + Duration::from_secs(NOW_SECS + 60),
        },
    );

    assert_eq!(
        return_target(&manager, 50_000),
        SendTarget::Resume(SessionId(1))
    );
}

// cost: time O(s), heap O(s), stack O(1), alloc 1
// vars: s = 테스트 session 수
// basis: estimate
fn states(manager: &SessionManager) -> Vec<SessionState> {
    manager
        .sessions
        .iter()
        .map(|session| session.state)
        .collect()
}

#[test]
fn provider_switches_keep_one_open_and_one_archive_per_provider() {
    let mut manager = manager_with(vec![record(1, Provider::Codex, SessionState::Open)]);
    manager
        .set_state(SessionId(1), SessionState::ClosedResumable)
        .unwrap();
    manager
        .register(record(2, Provider::Claude, SessionState::Open))
        .unwrap();
    manager
        .set_state(SessionId(2), SessionState::ClosedResumable)
        .unwrap();
    manager
        .register(record(3, Provider::Codex, SessionState::Open))
        .unwrap();

    manager
        .set_state(SessionId(3), SessionState::ClosedResumable)
        .unwrap();

    assert_eq!(
        states(&manager),
        vec![
            SessionState::Ended,
            SessionState::ClosedResumable,
            SessionState::ClosedResumable
        ]
    );
}

#[test]
fn register_second_archive_ends_older_one_and_drops_its_last_turn() {
    let mut manager = manager_with(vec![record(
        1,
        Provider::Codex,
        SessionState::ClosedResumable,
    )]);
    manager.record_last_turn(SessionId(1), last_turn(1, 1));

    manager
        .register(record(2, Provider::Codex, SessionState::ClosedResumable))
        .unwrap();

    assert_eq!(
        states(&manager),
        vec![SessionState::Ended, SessionState::ClosedResumable]
    );
    assert_eq!(manager.last_turn(SessionId(1)), None);
}

#[test]
fn replace_with_archive_ends_older_archive_of_same_provider() {
    let mut manager = manager_with(vec![
        record(1, Provider::Codex, SessionState::ClosedResumable),
        record(2, Provider::Claude, SessionState::Open),
    ]);
    manager.mark_idle(SessionId(2), Instant::now());

    manager
        .replace(
            SessionId(2),
            record(3, Provider::Codex, SessionState::ClosedResumable),
        )
        .unwrap();

    assert_eq!(
        states(&manager),
        vec![
            SessionState::Ended,
            SessionState::Ended,
            SessionState::ClosedResumable
        ]
    );
}

#[test]
fn archive_cap_keeps_held_session_and_other_provider() {
    let mut manager = manager_with(vec![
        record(1, Provider::Codex, SessionState::Held),
        record(2, Provider::Claude, SessionState::ClosedResumable),
    ]);

    manager
        .register(record(3, Provider::Codex, SessionState::ClosedResumable))
        .unwrap();

    assert_eq!(
        states(&manager),
        vec![
            SessionState::Held,
            SessionState::ClosedResumable,
            SessionState::ClosedResumable
        ]
    );
}

#[test]
fn set_state_resume_beside_open_main_returns_error() {
    let mut manager = archived_codex(1, 1);

    let result = manager.set_state(SessionId(1), SessionState::Open);

    assert!(matches!(result, Err(SessionError::MainAlreadyOpen)));
}
