use std::time::{Duration, SystemTime};

use saturn_core::sessions::{AgentRole, LastTurn, SendTarget, SessionError, SessionRecord};
use saturn_protocol::ids::{AgentId, ChatId, LedgerSeq, Provider, ProviderSessionId, SessionId};
use saturn_protocol::state::SessionState;

use super::*;
use crate::sessions::SendRequest;

const CODEX: SessionId = SessionId(1);
const CLAUDE: SessionId = SessionId(2);

fn record(id: SessionId, chat: ChatId, provider: Provider, state: SessionState) -> SessionRecord {
    SessionRecord {
        id,
        chat,
        agent: AgentId(1),
        role: AgentRole::Main,
        provider,
        provider_session: Some(ProviderSessionId(format!("p{}", id.0))),
        model: None,
        state,
        delivered: LedgerSeq(0),
        idle_since: None,
    }
}

fn last_turn(active: u64, ended_ago: Duration, now: SystemTime) -> LastTurn {
    LastTurn {
        active,
        ended_at: now - ended_ago,
    }
}

fn to_codex(engine: &Engine, chat: ChatId, packet: u64) -> SendRequest {
    SendRequest {
        chat,
        provider: Provider::Codex,
        role: AgentRole::Main,
        packet,
        settings: engine.settings.current().unwrap(),
    }
}

fn new_codex() -> SendTarget {
    SendTarget::New {
        provider: Provider::Codex,
        role: AgentRole::Main,
    }
}

/// Codex를 쓰다 Claude로 바꾼 채팅. Codex 메인은 보관하고 마지막 턴 값을 남긴다.
async fn switched_to_claude(engine: &mut Engine, workdir: &Path, last: LastTurn) -> ChatId {
    let chat = engine
        .store
        .create_chat(workdir.to_path_buf())
        .await
        .unwrap();
    engine
        .store
        .upsert_session(&record(CODEX, chat, Provider::Codex, SessionState::Open))
        .await
        .unwrap();
    engine
        .sessions
        .register(record(CODEX, chat, Provider::Codex, SessionState::Open))
        .unwrap();
    engine.finish_turn(CODEX, last).await.unwrap();
    engine.archive_main(CODEX).await.unwrap();
    engine
        .register_session(record(CLAUDE, chat, Provider::Claude, SessionState::Open))
        .await
        .unwrap();
    chat
}

/// 같은 판정을 재시작 전후에 구하고, 재시작 뒤 값을 돌려준다.
async fn target_before_and_after_restart(
    active: u64,
    ended_ago: Duration,
    packet: u64,
) -> (SendTarget, SendTarget) {
    let fixture = Fixture::new();
    let now = SystemTime::now();
    let mut engine = fixture.ready().await;
    let chat = switched_to_claude(
        &mut engine,
        &fixture.workdir,
        last_turn(active, ended_ago, now),
    )
    .await;
    let before = engine
        .send_target(to_codex(&engine, chat, packet), now)
        .await
        .unwrap();
    engine.shutdown().await.unwrap();

    let engine = fixture.ready().await;
    let after = engine
        .send_target(to_codex(&engine, chat, packet), now)
        .await
        .unwrap();
    (before, after)
}

#[tokio::test]
async fn restart_resumes_archived_session_within_cache_ttl() {
    let (before, after) =
        target_before_and_after_restart(100_000, Duration::from_secs(60), 50_000).await;

    assert_eq!(before, SendTarget::Resume(CODEX));
    assert_eq!(after, before);
}

#[tokio::test]
async fn restart_keeps_new_session_when_active_context_reaches_threshold() {
    let (before, after) =
        target_before_and_after_restart(200_000, Duration::from_secs(60), 50_000).await;

    assert_eq!(before, new_codex());
    assert_eq!(after, before);
}

#[tokio::test]
async fn restart_keeps_new_session_when_cache_expired_and_packet_is_smaller() {
    let (before, after) =
        target_before_and_after_restart(100_000, Duration::from_secs(600), 50_000).await;

    assert_eq!(before, new_codex());
    assert_eq!(after, before);
}

#[tokio::test]
async fn restart_resumes_when_cache_expired_and_packet_is_not_smaller() {
    let (before, after) =
        target_before_and_after_restart(100_000, Duration::from_secs(600), 150_000).await;

    assert_eq!(before, SendTarget::Resume(CODEX));
    assert_eq!(after, before);
}

#[tokio::test]
async fn restart_without_last_turn_resumes_archived_session() {
    let fixture = Fixture::new();
    let engine = fixture.ready().await;
    let chat = engine
        .store
        .create_chat(fixture.workdir.clone())
        .await
        .unwrap();
    for (id, provider, state) in [
        (CODEX, Provider::Codex, SessionState::ClosedResumable),
        (CLAUDE, Provider::Claude, SessionState::Open),
    ] {
        engine
            .store
            .upsert_session(&record(id, chat, provider, state))
            .await
            .unwrap();
    }
    engine.shutdown().await.unwrap();

    let engine = fixture.ready().await;
    let target = engine
        .send_target(to_codex(&engine, chat, 1), SystemTime::now())
        .await
        .unwrap();

    assert!(engine.sessions.last_turn(CODEX).is_none());
    assert_eq!(target, SendTarget::Resume(CODEX));
}

#[tokio::test]
async fn finish_turn_for_missing_session_leaves_sessions_untouched() {
    let fixture = Fixture::new();
    let mut engine = fixture.ready().await;
    let last = last_turn(10, Duration::ZERO, SystemTime::now());

    let error = engine.finish_turn(SessionId(9), last).await.unwrap_err();

    assert!(matches!(error, EngineError::Store(_)));
    assert!(engine.sessions.last_turn(SessionId(9)).is_none());
}

#[tokio::test]
async fn archive_main_ends_older_archive_and_saves_both_states() {
    let fixture = Fixture::new();
    let mut engine = fixture.ready().await;
    let now = SystemTime::now();
    let chat = switched_to_claude(
        &mut engine,
        &fixture.workdir,
        last_turn(1, Duration::ZERO, now),
    )
    .await;
    let newer_codex = SessionId(3);
    engine.archive_main(CLAUDE).await.unwrap();
    engine
        .register_session(record(
            newer_codex,
            chat,
            Provider::Codex,
            SessionState::Open,
        ))
        .await
        .unwrap();

    engine.archive_main(newer_codex).await.unwrap();

    let saved: Vec<_> = engine
        .store
        .sessions(chat)
        .await
        .unwrap()
        .into_iter()
        .map(|record| (record.id, record.state))
        .collect();
    assert_eq!(
        saved,
        vec![
            (CODEX, SessionState::Ended),
            (CLAUDE, SessionState::ClosedResumable),
            (newer_codex, SessionState::ClosedResumable),
        ]
    );
    assert!(engine.sessions.last_turn(CODEX).is_none());
}

#[tokio::test]
async fn resume_main_opens_archive_and_returns_delivered_number() {
    let fixture = Fixture::new();
    let mut engine = fixture.ready().await;
    let now = SystemTime::now();
    let chat = switched_to_claude(
        &mut engine,
        &fixture.workdir,
        last_turn(1, Duration::ZERO, now),
    )
    .await;
    engine.sessions.mark_delivered(CODEX, LedgerSeq(5));
    let early = engine.resume_main(CODEX).await.unwrap_err();
    engine.archive_main(CLAUDE).await.unwrap();

    let from = engine.resume_main(CODEX).await.unwrap();

    assert!(matches!(
        early,
        EngineError::Session(SessionError::MainAlreadyOpen)
    ));
    assert_eq!(from, LedgerSeq(5));
    let states: Vec<_> = engine
        .store
        .sessions(chat)
        .await
        .unwrap()
        .into_iter()
        .map(|record| record.state)
        .collect();
    assert_eq!(
        states,
        vec![SessionState::Open, SessionState::ClosedResumable]
    );
}

#[tokio::test]
async fn restore_keeps_open_main_left_by_a_crash() {
    let fixture = Fixture::new();
    let engine = fixture.ready().await;
    let chat = engine
        .store
        .create_chat(fixture.workdir.clone())
        .await
        .unwrap();
    engine
        .store
        .upsert_session(&record(CLAUDE, chat, Provider::Claude, SessionState::Open))
        .await
        .unwrap();
    engine.shutdown().await.unwrap();

    let engine = fixture.ready().await;

    assert_eq!(
        engine.sessions.get(CLAUDE).map(|record| record.state),
        Some(SessionState::Open)
    );
}
