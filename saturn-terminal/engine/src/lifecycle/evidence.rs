//! 근거 검색과 원문 조회 테스트(#539): 에이전트 작업이 출입증으로 자기 채팅의 기록만 찾고 원문을 다시 읽는다.

use saturn_core::sessions::evidence::text_hash;
use saturn_protocol::ids::{ChatId, LedgerSeq};
use saturn_protocol::rpc::{EvidenceItem, QueryResult};

use super::child_passes::{Family, family};
use super::support::{tool_read, tool_result};
use crate::{EngineError, EvidenceRefusal};

const PASS_FREE_CONFIG: &str = "[permission.read]\n\"*/vault/*\" = \"deny\"\n";

fn items_of(result: QueryResult) -> (String, u32, Vec<EvidenceItem>) {
    match result {
        QueryResult::EvidenceCandidates {
            set_hash,
            total,
            items,
        } => (set_hash, total, items),
        other => panic!("not a candidate list: {other:?}"),
    }
}

fn refusal_of(error: EngineError) -> EvidenceRefusal {
    match error {
        EngineError::Evidence { refusal } => refusal,
        other => panic!("not an evidence refusal: {other:?}"),
    }
}

/// 첫 도구 호출이 `login`을 다루고 둘째가 `alpha`를 다루는 채팅의 후보 번호 둘.
async fn two_records(family: &mut Family) -> (LedgerSeq, LedgerSeq) {
    let agent = family.parent_agent;
    family
        .flow
        .claude_event(tool_read(agent, "c1", "src/auth.rs"))
        .await;
    family
        .flow
        .claude_event(tool_result(agent, "c1", "login failure message handler"))
        .await;
    family
        .flow
        .claude_event(tool_read(agent, "c2", "README.md"))
        .await;
    family
        .flow
        .claude_event(tool_result(agent, "c2", "alpha beta gamma"))
        .await;
    let (_, _, items) = items_of(
        family
            .flow
            .engine
            .evidence_search(&family.pass, "login", 10)
            .await
            .unwrap(),
    );
    assert_eq!(items.len(), 2);
    (items[0].id, items[1].id)
}

#[tokio::test]
async fn search_ranks_by_the_query_and_read_returns_the_recorded_text_in_pages() {
    let mut family = family("", 0).await;
    let (login, other) = two_records(&mut family).await;
    let engine = &family.flow.engine;

    let (set_hash, total, items) = items_of(
        engine
            .evidence_search(&family.pass, "login", 1)
            .await
            .unwrap(),
    );
    let first = engine
        .evidence_read(&family.pass, (login, Some(items[0].hash.as_str())), (0, 5))
        .await
        .unwrap();
    let rest = engine
        .evidence_read(&family.pass, (login, None), (5, 10_000))
        .await
        .unwrap();

    assert_eq!((items.len(), total, set_hash.len()), (1, 2, 64));
    assert_eq!(items[0].id, login);
    assert_ne!(login, other);
    let QueryResult::EvidenceRecord {
        text: head,
        next_offset,
        hash,
        ..
    } = first
    else {
        panic!("not a record");
    };
    let QueryResult::EvidenceRecord {
        text: tail,
        next_offset: end,
        chars,
        ..
    } = rest
    else {
        panic!("not a record");
    };
    let full = format!("{head}{tail}");
    assert_eq!(next_offset, Some(5));
    assert_eq!(end, None);
    assert!(full.contains("login failure message handler"));
    assert_eq!(
        (full.chars().count() as u64, hash),
        (chars, text_hash(&full))
    );
}

#[tokio::test]
async fn read_refuses_unknown_other_chat_stale_and_out_of_scope_records() {
    let mut family = family(PASS_FREE_CONFIG, 1).await;
    let (login, _) = two_records(&mut family).await;
    let agent = family.parent_agent;
    family
        .flow
        .claude_event(tool_read(agent, "c3", "/etc/hosts"))
        .await;
    family
        .flow
        .claude_event(tool_result(agent, "c3", "127.0.0.1 localhost"))
        .await;
    family
        .flow
        .claude_event(tool_read(agent, "c4", "vault/keys.txt"))
        .await;
    family
        .flow
        .claude_event(tool_result(agent, "c4", "vault contents"))
        .await;
    let other = family.flow.open_other_chat().await;
    family
        .flow
        .event(
            crate::providers::test_support::CLAUDE,
            tool_read(other.agent, "o1", "notes.md"),
        )
        .await;
    family
        .flow
        .event(
            crate::providers::test_support::CLAUDE,
            tool_result(other.agent, "o1", "other chat secret"),
        )
        .await;
    let engine = &family.flow.engine;
    let ledger =
        |chat: ChatId| async move { engine.store.ledger_since(chat, LedgerSeq(0)).await.unwrap() };
    let outside = ledger(family.flow.chat)
        .await
        .iter()
        .find(|row| matches!(&row.event, saturn_protocol::event::ProviderEvent::ToolCall { call_id, .. } if call_id == "c3"))
        .map(|row| row.seq)
        .unwrap();
    let vault = ledger(family.flow.chat)
        .await
        .iter()
        .find(|row| matches!(&row.event, saturn_protocol::event::ProviderEvent::ToolCall { call_id, .. } if call_id == "c4"))
        .map(|row| row.seq)
        .unwrap();
    let foreign = ledger(other.chat)
        .await
        .iter()
        .find(|row| matches!(&row.event, saturn_protocol::event::ProviderEvent::ToolCall { call_id, .. } if call_id == "o1"))
        .map(|row| row.seq)
        .unwrap();
    let rows: [(LedgerSeq, Option<&str>, EvidenceRefusal); 4] = [
        (LedgerSeq(9_999), None, EvidenceRefusal::NotFound),
        (login, Some("an-older-hash"), EvidenceRefusal::Stale),
        (outside, None, EvidenceRefusal::Scope),
        (vault, None, EvidenceRefusal::Scope),
    ];

    for (id, hash, expected) in rows {
        let error = engine
            .evidence_read(&family.pass, (id, hash), (0, 100))
            .await
            .unwrap_err();

        assert_eq!(refusal_of(error), expected, "{id:?}");
    }
    let (_, total, items) = items_of(
        engine
            .evidence_search(&family.pass, "contents hosts secret", 50)
            .await
            .unwrap(),
    );
    assert_eq!(total, 2, "only in-scope records are candidates");
    assert!(
        items
            .iter()
            .all(|item| ![outside, vault].contains(&item.id))
    );
    // 기록 번호는 채팅마다 센다. 다른 채팅에서 같은 번호를 읽어도 이 채팅의 기록이 나오고 다른 채팅의 글은 나오지 않는다
    let QueryResult::EvidenceRecord { text, .. } = engine
        .evidence_read(&family.pass, (foreign, None), (0, 1_000))
        .await
        .unwrap()
    else {
        panic!("not a record");
    };
    assert!(!text.contains("other chat secret"), "{text}");
}

#[tokio::test]
async fn an_unknown_pass_is_refused_and_every_lookup_is_counted() {
    let mut family = family("", 0).await;
    let (login, _) = two_records(&mut family).await;
    let engine = &family.flow.engine;

    let search = engine.evidence_search("saturn-pass-nope", "x", 5).await;
    let read = engine
        .evidence_read("saturn-pass-nope", (login, None), (0, 5))
        .await;
    engine
        .evidence_read(&family.pass, (login, None), (0, 5))
        .await
        .unwrap();
    engine
        .evidence_read(&family.pass, (LedgerSeq(9_999), None), (0, 5))
        .await
        .unwrap_err();

    assert_eq!(refusal_of(search.unwrap_err()), EvidenceRefusal::Pass);
    assert_eq!(refusal_of(read.unwrap_err()), EvidenceRefusal::Pass);
    let lookups = engine
        .store
        .evidence_lookups(family.flow.chat)
        .await
        .unwrap();
    let summary: Vec<(&str, &str)> = lookups
        .iter()
        .map(|lookup| (lookup.kind.as_str(), lookup.outcome.as_str()))
        .collect();
    assert_eq!(
        summary,
        [("Search", "Ok"), ("Read", "Ok"), ("Read", "NotFound")]
    );
    assert_eq!(lookups[1].record, Some(login));
    assert_eq!(lookups[1].units, 5);
}
