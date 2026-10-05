use super::*;
use crate::sessions::context::{
    DEFAULT_CONSTRAINT_SLOT_PERCENT, DEFAULT_ITEM_CAP_PERCENT, DEFAULT_PACKET_HARD_PERCENT,
};
use crate::sessions::ranking::DEFAULT_RRF_K;

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
            packet_hard_percent: DEFAULT_PACKET_HARD_PERCENT,
            item_cap_percent: DEFAULT_ITEM_CAP_PERCENT,
            constraint_slot_percent: DEFAULT_CONSTRAINT_SLOT_PERCENT,
            rrf_k: DEFAULT_RRF_K,
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
        record(
            1,
            Provider::from_static("codex"),
            SessionState::ClosedResumable,
        ),
        record(2, Provider::from_static("claude"), SessionState::Open),
    ]);
    manager.record_last_turn(SessionId(1), last_turn(active, secs_ago));
    manager
}

fn return_target(manager: &SessionManager, packet: u64) -> SendTarget {
    manager.target_for_send(
        CHAT,
        Provider::from_static("codex"),
        AgentRole::Main,
        &inputs(packet),
    )
}

fn new_codex() -> SendTarget {
    SendTarget::New {
        provider: Provider::from_static("codex"),
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
fn target_for_send_picks_the_session_to_use() {
    let claude = Provider::from_static("claude");
    let codex = Provider::from_static("codex");
    let new = |provider: &Provider, role| SendTarget::New {
        provider: *provider,
        role,
    };
    let no_provider_id = {
        let mut closed = record(1, codex, SessionState::ClosedResumable);
        closed.provider_session = None;
        closed
    };
    // (사례, 등록한 session, 대상 provider, 역할, 예상 대상)
    let cases = [
        (
            "without session",
            vec![],
            claude,
            AgentRole::Main,
            new(&claude, AgentRole::Main),
        ),
        (
            "open same provider",
            vec![record(1, claude, SessionState::Open)],
            claude,
            AgentRole::Main,
            SendTarget::Open(SessionId(1)),
        ),
        (
            "closed session",
            vec![record(1, codex, SessionState::ClosedResumable)],
            codex,
            AgentRole::Main,
            SendTarget::Resume(SessionId(1)),
        ),
        (
            "closed without provider id",
            vec![no_provider_id],
            codex,
            AgentRole::Main,
            new(&codex, AgentRole::Main),
        ),
        (
            "other provider",
            vec![record(1, claude, SessionState::Open)],
            codex,
            AgentRole::Main,
            new(&codex, AgentRole::Main),
        ),
        (
            "sub always new",
            vec![record(1, claude, SessionState::Open)],
            claude,
            AgentRole::Sub,
            new(&claude, AgentRole::Sub),
        ),
    ];

    for (name, records, provider, role, expected) in cases {
        let manager = manager_with(records);

        let target = manager.target_for_send(CHAT, provider, role, &inputs(50_000));

        assert_eq!(target, expected, "{name}");
    }
}

#[test]
fn register_second_open_main_returns_error() {
    let mut manager = manager_with(vec![record(
        1,
        Provider::from_static("claude"),
        SessionState::Open,
    )]);

    let result = manager.register(record(
        2,
        Provider::from_static("codex"),
        SessionState::Open,
    ));

    assert!(matches!(result, Err(SessionError::MainAlreadyOpen)));
}

#[test]
fn register_duplicate_id_returns_error() {
    let mut manager = manager_with(vec![record(
        1,
        Provider::from_static("claude"),
        SessionState::Ended,
    )]);

    let result = manager.register(record(
        1,
        Provider::from_static("codex"),
        SessionState::Open,
    ));

    assert!(matches!(
        result,
        Err(SessionError::DuplicateId(SessionId(1)))
    ));
}

#[test]
fn register_sub_beside_main_succeeds() {
    let mut manager = manager_with(vec![record(
        1,
        Provider::from_static("claude"),
        SessionState::Open,
    )]);
    let mut sub = record(2, Provider::from_static("codex"), SessionState::Open);
    sub.role = AgentRole::Sub;

    let result = manager.register(sub);

    assert!(result.is_ok());
}

#[test]
fn replace_during_turn_returns_not_at_turn_boundary() {
    let mut manager = manager_with(vec![record(
        1,
        Provider::from_static("claude"),
        SessionState::Open,
    )]);

    let result = manager.replace(
        SessionId(1),
        record(2, Provider::from_static("codex"), SessionState::Open),
    );

    assert!(matches!(result, Err(SessionError::NotAtTurnBoundary)));
}

// cost: time O(s), heap O(1), stack O(1)
// vars: s = 테스트 session 수
// basis: estimate
#[test]
fn replace_keeps_one_live_session_per_chat() {
    let mut manager = manager_with(vec![record(
        1,
        Provider::from_static("claude"),
        SessionState::Open,
    )]);
    manager.mark_idle(SessionId(1), Instant::now());

    manager
        .replace(
            SessionId(1),
            record(2, Provider::from_static("codex"), SessionState::Open),
        )
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
    let mut manager = manager_with(vec![record(
        1,
        Provider::from_static("claude"),
        SessionState::Open,
    )]);
    manager.mark_delivered(SessionId(1), LedgerSeq(40));
    manager.mark_idle(SessionId(1), Instant::now());

    manager
        .replace(
            SessionId(1),
            record(2, Provider::from_static("codex"), SessionState::Open),
        )
        .unwrap();

    assert_eq!(manager.attach_from(SessionId(2)), LedgerSeq(40));
}

#[test]
fn replace_keeps_packet_seq_when_newer() {
    let mut manager = manager_with(vec![record(
        1,
        Provider::from_static("claude"),
        SessionState::Held,
    )]);
    manager.mark_delivered(SessionId(1), LedgerSeq(40));
    let mut new = record(2, Provider::from_static("codex"), SessionState::Open);
    new.delivered = LedgerSeq(55);

    manager.replace(SessionId(1), new).unwrap();

    assert_eq!(manager.attach_from(SessionId(2)), LedgerSeq(55));
}

#[test]
fn replace_unknown_session_returns_not_found() {
    let mut manager = SessionManager::new();

    let result = manager.replace(
        SessionId(9),
        record(2, Provider::from_static("codex"), SessionState::Open),
    );

    assert!(matches!(result, Err(SessionError::NotFound(SessionId(9)))));
}

#[test]
fn replace_other_chat_keeps_original_session() {
    let mut manager = manager_with(vec![record(
        1,
        Provider::from_static("claude"),
        SessionState::Held,
    )]);
    let mut other_chat = record(2, Provider::from_static("codex"), SessionState::Open);
    other_chat.chat = ChatId(2);

    let result = manager.replace(SessionId(1), other_chat);

    assert!(matches!(result, Err(SessionError::InvalidReplacement)));
    assert_eq!(manager.get(SessionId(1)).unwrap().state, SessionState::Held);
}

#[test]
fn replace_duplicate_id_keeps_original_session() {
    let mut manager = manager_with(vec![record(
        1,
        Provider::from_static("claude"),
        SessionState::Held,
    )]);

    let result = manager.replace(
        SessionId(1),
        record(1, Provider::from_static("codex"), SessionState::Open),
    );

    assert!(matches!(
        result,
        Err(SessionError::DuplicateId(SessionId(1)))
    ));
    assert_eq!(manager.get(SessionId(1)).unwrap().state, SessionState::Held);
}

#[test]
fn mark_delivered_never_lowers() {
    let mut manager = manager_with(vec![record(
        1,
        Provider::from_static("claude"),
        SessionState::Open,
    )]);
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
    let mut manager = manager_with(vec![record(
        1,
        Provider::from_static("claude"),
        SessionState::Open,
    )]);

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
    let mut manager = manager_with(vec![record(
        1,
        Provider::from_static("claude"),
        SessionState::Ended,
    )]);

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
        Provider::from_static("claude"),
        SessionState::ClosedResumable,
    )]);

    let result = manager.set_state(SessionId(1), SessionState::Held);

    assert!(result.is_err());
}

#[test]
fn target_for_send_decides_returning_to_the_archive_by_cache_and_packet_size() {
    // (사례, 활성 크기, 마지막 턴 뒤 경과 초, 패킷 크기, 예상 대상)
    let cases = [
        (
            "warm below threshold",
            99_999,
            300,
            50_000,
            SendTarget::Resume(SessionId(1)),
        ),
        (
            "warm above threshold",
            150_000,
            10,
            50_000,
            SendTarget::Resume(SessionId(1)),
        ),
        ("expired smaller packet", 80_000, 301, 79_999, new_codex()),
        (
            "expired packet not smaller",
            80_000,
            301,
            80_000,
            SendTarget::Resume(SessionId(1)),
        ),
    ];

    for (name, active, secs_ago, packet, expected) in cases {
        let manager = archived_codex(active, secs_ago);

        assert_eq!(return_target(&manager, packet), expected, "{name}");
    }
}

#[test]
fn target_for_send_archive_without_last_turn_resumes() {
    let manager = manager_with(vec![
        record(
            1,
            Provider::from_static("codex"),
            SessionState::ClosedResumable,
        ),
        record(2, Provider::from_static("claude"), SessionState::Open),
    ]);

    assert_eq!(
        return_target(&manager, 50_000),
        SendTarget::Resume(SessionId(1))
    );
}

#[test]
fn live_main_prefers_the_open_main_over_a_later_registered_archive() {
    let manager = manager_with(vec![
        record(1, Provider::from_static("codex"), SessionState::Open),
        record(
            2,
            Provider::from_static("claude"),
            SessionState::ClosedResumable,
        ),
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
    let mut archive = record(
        1,
        Provider::from_static("codex"),
        SessionState::ClosedResumable,
    );
    archive.provider_session = None;
    let manager = manager_with(vec![
        archive,
        record(2, Provider::from_static("claude"), SessionState::Open),
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
    let mut manager = manager_with(vec![record(
        1,
        Provider::from_static("codex"),
        SessionState::Open,
    )]);
    manager
        .set_state(SessionId(1), SessionState::ClosedResumable)
        .unwrap();
    manager
        .register(record(
            2,
            Provider::from_static("claude"),
            SessionState::Open,
        ))
        .unwrap();
    manager
        .set_state(SessionId(2), SessionState::ClosedResumable)
        .unwrap();
    manager
        .register(record(
            3,
            Provider::from_static("codex"),
            SessionState::Open,
        ))
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
        Provider::from_static("codex"),
        SessionState::ClosedResumable,
    )]);
    manager.record_last_turn(SessionId(1), last_turn(1, 1));

    manager
        .register(record(
            2,
            Provider::from_static("codex"),
            SessionState::ClosedResumable,
        ))
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
        record(
            1,
            Provider::from_static("codex"),
            SessionState::ClosedResumable,
        ),
        record(2, Provider::from_static("claude"), SessionState::Open),
    ]);
    manager.mark_idle(SessionId(2), Instant::now());

    manager
        .replace(
            SessionId(2),
            record(
                3,
                Provider::from_static("codex"),
                SessionState::ClosedResumable,
            ),
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
        record(1, Provider::from_static("codex"), SessionState::Held),
        record(
            2,
            Provider::from_static("claude"),
            SessionState::ClosedResumable,
        ),
    ]);

    manager
        .register(record(
            3,
            Provider::from_static("codex"),
            SessionState::ClosedResumable,
        ))
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

#[test]
fn idle_expired_counts_only_idle_open_mains_past_the_grace_with_an_idle_tree() {
    let claude = Provider::from_static("claude");
    let start = Instant::now();
    let mut manager = manager_with(vec![
        record(1, claude, SessionState::Open),
        {
            let mut other = record(2, claude, SessionState::Open);
            other.chat = ChatId(CHAT.0 + 1);
            other
        },
        {
            let mut sub = record(3, claude, SessionState::Open);
            sub.chat = ChatId(CHAT.0 + 2);
            sub.role = AgentRole::Sub;
            sub
        },
    ]);
    for id in [1, 2, 3] {
        manager.mark_idle(SessionId(id), start);
    }
    let all_idle = |_: AgentId| true;

    let before = start + IDLE_GRACE - Duration::from_secs(1);
    assert!(
        manager
            .idle_expired(before, IDLE_GRACE, all_idle)
            .is_empty()
    );
    let at = start + IDLE_GRACE;
    assert_eq!(
        manager.idle_expired(at, IDLE_GRACE, all_idle),
        vec![SessionId(1), SessionId(2)]
    );
    assert_eq!(
        manager.idle_expired(at, IDLE_GRACE, |agent| agent != AgentId(1)),
        vec![SessionId(2)]
    );
    manager.mark_busy(SessionId(1));
    assert_eq!(
        manager.idle_expired(at, IDLE_GRACE, all_idle),
        vec![SessionId(2)]
    );
    manager
        .set_state(SessionId(2), SessionState::ClosedResumable)
        .unwrap();
    assert!(manager.idle_expired(at, IDLE_GRACE, all_idle).is_empty());
}

#[test]
fn undoing_an_idle_close_restores_the_open_state_and_the_idle_clock() {
    let claude = Provider::from_static("claude");
    let start = Instant::now();
    let mut manager = manager_with(vec![record(1, claude, SessionState::Open)]);
    manager.mark_idle(SessionId(1), start);
    manager
        .set_state(SessionId(1), SessionState::ClosedResumable)
        .unwrap();
    assert!(manager.get(SessionId(1)).unwrap().idle_since.is_none());

    manager.undo_idle_close(SessionId(1), Some(start)).unwrap();

    let restored = manager.get(SessionId(1)).unwrap();
    assert_eq!(restored.state, SessionState::Open);
    assert_eq!(restored.idle_since, Some(start));
    assert_eq!(
        manager.idle_expired(start + IDLE_GRACE, IDLE_GRACE, |_| true),
        vec![SessionId(1)]
    );
}
