//! 근거 검색과 원문 조회 테스트(#539): 에이전트 작업이 출입증으로 자기 채팅의 기록만 찾고 원문을 다시 읽는다.

use saturn_core::sessions::evidence::text_hash;
use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::SubagentId;
use saturn_protocol::ids::{AgentId, ChatId, LedgerSeq};
use saturn_protocol::rpc::{EvidenceItem, EvidenceKind, QueryResult};

use super::child_passes::{Family, family};
use super::support::{subagent_started, text, tool_read, tool_result};
use crate::evidence::EvidenceTarget;
use crate::providers::test_support::CLAUDE;
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
    let tools = of_kind(&items, EvidenceKind::Tool);
    assert_eq!(tools.len(), 2);
    (tools[0].id, tools[1].id)
}

fn of_kind(items: &[EvidenceItem], kind: EvidenceKind) -> Vec<&EvidenceItem> {
    items.iter().filter(|item| item.kind == kind).collect()
}

/// 검색에서 본 후보를 종류, 채팅, 번호, 해시로 가리키는 읽기 지정.
fn typed(chat: ChatId, item: &EvidenceItem) -> EvidenceTarget<'_> {
    EvidenceTarget {
        kind: Some(item.kind),
        chat: Some(chat),
        id: item.id,
        hash: Some(item.hash.as_str()),
    }
}

/// 검색어에 상관없이 후보 전부(50개 안)와 후보 수.
async fn everything(family: &Family) -> (u32, Vec<EvidenceItem>) {
    let (_, total, items) = items_of(
        family
            .flow
            .engine
            .evidence_search(&family.pass, "login parent answer", 50)
            .await
            .unwrap(),
    );
    (total, items)
}

async fn body_of(family: &Family, target: EvidenceTarget<'_>) -> Result<String, EvidenceRefusal> {
    match family
        .flow
        .engine
        .evidence_read(&family.pass, target, (0, 10_000))
        .await
    {
        Ok(QueryResult::EvidenceRecord { text, .. }) => Ok(text),
        Ok(other) => panic!("not a record: {other:?}"),
        Err(error) => Err(refusal_of(error)),
    }
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
        .evidence_read(
            &family.pass,
            EvidenceTarget::tool(login, Some(items[0].hash.as_str())),
            (0, 5),
        )
        .await
        .unwrap();
    let rest = engine
        .evidence_read(&family.pass, EvidenceTarget::tool(login, None), (5, 10_000))
        .await
        .unwrap();
    let (_, everything_total, everything) = items_of(
        engine
            .evidence_search(&family.pass, "login", 50)
            .await
            .unwrap(),
    );

    // 후보는 도구 호출 둘에 이 채팅의 입력이 더해진다
    assert_eq!((items.len(), set_hash.len()), (1, 64));
    assert_eq!(
        (items[0].excerpt_start, items[0].excerpt_end),
        (0, items[0].excerpt.chars().count() as u64)
    );
    assert_eq!(total, everything_total);
    assert_eq!(total as usize, everything.len());
    assert_eq!(of_kind(&everything, EvidenceKind::Tool).len(), 2);
    assert_eq!((items[0].kind, items[0].id), (EvidenceKind::Tool, login));
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
            .evidence_read(&family.pass, EvidenceTarget::tool(id, hash), (0, 100))
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
    let tools = of_kind(&items, EvidenceKind::Tool);
    assert_eq!(tools.len(), 2, "only in-scope tool records are candidates");
    assert_eq!(total as usize, items.len());
    assert!(
        tools
            .iter()
            .all(|item| ![outside, vault].contains(&item.id))
    );
    // 기록 번호는 채팅마다 센다. 다른 채팅에서 같은 번호를 읽어도 이 채팅의 기록이 나오고 다른 채팅의 글은 나오지 않는다
    let QueryResult::EvidenceRecord { text, .. } = engine
        .evidence_read(
            &family.pass,
            EvidenceTarget::tool(foreign, None),
            (0, 1_000),
        )
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
        .evidence_read(
            "saturn-pass-nope",
            EvidenceTarget::tool(login, None),
            (0, 5),
        )
        .await;
    engine
        .evidence_read(&family.pass, EvidenceTarget::tool(login, None), (0, 5))
        .await
        .unwrap();
    engine
        .evidence_read(
            &family.pass,
            EvidenceTarget::tool(LedgerSeq(9_999), None),
            (0, 5),
        )
        .await
        .unwrap_err();
    // 명령이 engine에 닿지 못한 시도는 도구 결과의 오류 표지로 센다. 첫머리가 아닌 표지와 다른 오류는 세지 않는다
    for output in [
        "Error: saturn evidence unreachable (read 7): blocked\n\nCaused by:\n    0: Operation not permitted",
        "Error: saturn evidence unreachable (search): blocked",
        "file text\nError: saturn evidence unreachable (read 3): blocked",
        "Error: saturn evidence unreachable (read x): blocked",
        "Error: something else",
    ] {
        let result = tool_result(AgentId(1), "c", output);
        engine
            .note_unreachable_lookup(family.flow.chat, &result)
            .await;
    }

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
        [
            ("Search", "Ok"),
            ("Read", "Ok"),
            ("Read", "NotFound"),
            ("Read", "Unreachable"),
            ("Search", "Unreachable"),
        ]
    );
    assert_eq!(lookups[3].record, Some(LedgerSeq(7)));
    assert_eq!(lookups[1].record, Some(login));
    assert_eq!(lookups[1].units, 5);
}

#[tokio::test]
async fn an_input_and_an_answer_with_the_same_number_are_read_by_their_own_kind() {
    let mut family = family("", 0).await;
    let agent = family.parent_agent;
    let chat = family.flow.chat;
    // 실행의 첫 기록이 글이면 실행을 연 입력과 그 글이 같은 번호를 쓴다
    family
        .flow
        .claude_event(text(agent, "the login answer"))
        .await;
    family
        .flow
        .claude_event(subagent_started(agent, "s1", None))
        .await;
    family
        .flow
        .claude_event(ProviderEvent::Text {
            agent,
            subagent: Some(SubagentId("s1".to_owned())),
            text: "subagent zebra".to_owned(),
        })
        .await;

    let (total, items) = everything(&family).await;
    let inputs = of_kind(&items, EvidenceKind::Input);
    let answers = of_kind(&items, EvidenceKind::Text);

    assert_eq!((inputs.len(), answers.len()), (1, 1));
    assert_eq!(total as usize, items.len());
    assert!(
        items.iter().all(|item| !item.excerpt.contains("zebra")),
        "subagent text is not a candidate"
    );
    let (input, answer) = (inputs[0], answers[0]);
    assert_eq!(input.id, answer.id);
    assert_ne!(input.hash, answer.hash);
    assert_eq!((input.chat, answer.chat), (Some(chat), Some(chat)));
    assert_eq!(
        body_of(&family, typed(chat, input)).await.as_deref(),
        Ok("parent task")
    );
    assert_eq!(
        body_of(&family, typed(chat, answer)).await.as_deref(),
        Ok("the login answer")
    );
    // 같은 번호에 다른 종류의 해시를 대면 다른 원문이라 거절하고, 옛 형식의 번호는 도구 호출만 가리킨다
    let crossed = EvidenceTarget {
        hash: Some(answer.hash.as_str()),
        ..typed(chat, input)
    };
    assert_eq!(body_of(&family, crossed).await, Err(EvidenceRefusal::Stale));
    assert_eq!(
        body_of(&family, EvidenceTarget::tool(input.id, None)).await,
        Err(EvidenceRefusal::NotFound)
    );
}

#[tokio::test]
async fn an_input_and_a_tool_record_with_the_same_number_stay_apart_and_legacy_reads_the_tool() {
    let mut family = family("", 0).await;
    let agent = family.parent_agent;
    let chat = family.flow.chat;
    // 실행의 첫 기록이 도구 호출이면 실행을 연 입력과 그 호출이 같은 번호를 쓴다
    family
        .flow
        .claude_event(tool_read(agent, "c1", "src/auth.rs"))
        .await;
    family
        .flow
        .claude_event(tool_result(agent, "c1", "login failure message"))
        .await;

    let (_, items) = everything(&family).await;
    let (tool, input) = (
        of_kind(&items, EvidenceKind::Tool)[0],
        of_kind(&items, EvidenceKind::Input)[0],
    );

    assert_eq!(tool.id, input.id);
    let legacy = body_of(&family, EvidenceTarget::tool(tool.id, Some(&tool.hash))).await;
    assert!(legacy.unwrap().contains("login failure message"));
    let typed_tool = body_of(&family, typed(chat, tool)).await.unwrap();
    assert!(typed_tool.contains("login failure message"));
    assert_eq!(
        body_of(&family, typed(chat, input)).await.as_deref(),
        Ok("parent task")
    );
}

#[tokio::test]
async fn a_changed_text_or_a_missing_hash_is_refused_as_stale() {
    let mut family = family("", 0).await;
    let agent = family.parent_agent;
    let chat = family.flow.chat;
    family
        .flow
        .claude_event(tool_read(agent, "c1", "src/auth.rs"))
        .await;
    family.flow.claude_event(text(agent, "an answer")).await;
    let (_, before) = everything(&family).await;
    let tool = of_kind(&before, EvidenceKind::Tool)[0].clone();
    let answer = of_kind(&before, EvidenceKind::Text)[0].clone();
    assert!(body_of(&family, typed(chat, &tool)).await.is_ok());

    // 결과가 늦게 와 호출의 원문이 바뀌면 전에 본 해시로는 읽지 못한다
    family
        .flow
        .claude_event(tool_result(agent, "c1", "late result"))
        .await;

    assert_eq!(
        body_of(&family, typed(chat, &tool)).await,
        Err(EvidenceRefusal::Stale)
    );
    assert_eq!(
        body_of(&family, EvidenceTarget::tool(tool.id, Some(&tool.hash))).await,
        Err(EvidenceRefusal::Stale)
    );
    let wrong = EvidenceTarget {
        hash: Some("an-older-hash"),
        ..typed(chat, &answer)
    };
    let unhashed = EvidenceTarget {
        hash: None,
        ..typed(chat, &answer)
    };
    assert_eq!(body_of(&family, wrong).await, Err(EvidenceRefusal::Stale));
    assert_eq!(
        body_of(&family, unhashed).await,
        Err(EvidenceRefusal::Stale)
    );
    assert_eq!(
        body_of(&family, EvidenceTarget::tool(tool.id, None))
            .await
            .map(|text| text.contains("late result")),
        Ok(true)
    );
}

#[tokio::test]
async fn a_reference_to_another_chat_is_refused_and_other_chat_dialogue_is_not_listed() {
    let mut family = family("", 1).await;
    let chat = family.flow.chat;
    let other = family.flow.open_other_chat().await;
    family
        .flow
        .event(CLAUDE, text(other.agent, "other chat zebra"))
        .await;
    let agent = family.parent_agent;
    family.flow.claude_event(text(agent, "mine")).await;

    let (_, items) = everything(&family).await;
    let mine = of_kind(&items, EvidenceKind::Text)[0];

    assert!(items.iter().all(|item| !item.excerpt.contains("zebra")));
    let elsewhere = EvidenceTarget {
        chat: Some(other.chat),
        ..typed(chat, mine)
    };
    let unscoped = EvidenceTarget {
        chat: None,
        ..typed(chat, mine)
    };
    assert_eq!(
        body_of(&family, elsewhere).await,
        Err(EvidenceRefusal::NotFound)
    );
    assert_eq!(
        body_of(&family, unscoped).await,
        Err(EvidenceRefusal::NotFound)
    );
    assert_eq!(
        body_of(&family, typed(chat, mine)).await.as_deref(),
        Ok("mine")
    );
}

#[tokio::test]
async fn a_narrowed_read_permission_refuses_a_reference_it_listed_before() {
    let mut family = family("", 0).await;
    let agent = family.parent_agent;
    let chat = family.flow.chat;
    family
        .flow
        .claude_event(tool_read(agent, "c1", "vault/keys.txt"))
        .await;
    family
        .flow
        .claude_event(tool_result(agent, "c1", "vault contents"))
        .await;
    family.flow.claude_event(text(agent, "plain answer")).await;
    let (_, before) = everything(&family).await;
    let tool = of_kind(&before, EvidenceKind::Tool)[0].clone();
    let answer = of_kind(&before, EvidenceKind::Text)[0].clone();
    assert!(body_of(&family, typed(chat, &tool)).await.is_ok());

    family.flow.fixture.write_user_config(PASS_FREE_CONFIG);
    family.flow.engine.watch_settings().await;
    family.flow.engine.watch_settings().await;

    assert_eq!(
        body_of(&family, typed(chat, &tool)).await,
        Err(EvidenceRefusal::Scope)
    );
    assert_eq!(
        body_of(&family, EvidenceTarget::tool(tool.id, None)).await,
        Err(EvidenceRefusal::Scope)
    );
    let (total, after) = everything(&family).await;
    assert!(of_kind(&after, EvidenceKind::Tool).is_empty());
    assert_eq!(total as usize, after.len());
    // 입력과 글은 파일을 가리키지 않아 파일 읽기 규칙이 좁아져도 읽힌다
    assert_eq!(
        body_of(&family, typed(chat, &answer)).await.as_deref(),
        Ok("plain answer")
    );
}

#[tokio::test]
async fn a_revoked_pass_refuses_search_and_typed_reads_alike() {
    let mut family = family("", 0).await;
    let agent = family.parent_agent;
    let chat = family.flow.chat;
    family.flow.claude_event(text(agent, "an answer")).await;
    let (_, items) = everything(&family).await;
    let answer = of_kind(&items, EvidenceKind::Text)[0].clone();
    assert!(body_of(&family, typed(chat, &answer)).await.is_ok());

    family.flow.engine.passes.end(chat);

    let search = family
        .flow
        .engine
        .evidence_search(&family.pass, "answer", 5)
        .await;
    assert_eq!(refusal_of(search.unwrap_err()), EvidenceRefusal::Pass);
    assert_eq!(
        body_of(&family, typed(chat, &answer)).await,
        Err(EvidenceRefusal::Pass)
    );
    assert_eq!(
        body_of(&family, EvidenceTarget::tool(answer.id, None)).await,
        Err(EvidenceRefusal::Pass)
    );
}

#[tokio::test]
async fn dialogue_with_a_router_key_is_neither_listed_nor_readable() {
    let mut family = family("", 0).await;
    let agent = family.parent_agent;
    let chat = family.flow.chat;
    family.flow.engine.masker = crate::Masker::new(vec!["sk-leak-123".to_owned()]);
    family
        .flow
        .claude_event(text(agent, "the key is sk-leak-123"))
        .await;
    family.flow.claude_event(text(agent, "a safe answer")).await;

    let (_, items) = everything(&family).await;
    let answers = of_kind(&items, EvidenceKind::Text);

    assert_eq!(answers.len(), 1);
    assert!(items.iter().all(|item| !item.excerpt.contains("sk-leak")));
    // 비밀이 든 글의 번호와 해시를 알아도 읽지 못하고, 있다는 사실도 알리지 않는다
    let leaked = "the key is sk-leak-123";
    let id = family
        .flow
        .engine
        .store
        .ledger_since(chat, LedgerSeq(0))
        .await
        .unwrap()
        .iter()
        .find(|row| matches!(&row.event, ProviderEvent::Text { text, .. } if text == leaked))
        .map(|row| row.seq)
        .unwrap();
    let hash = text_hash(leaked);
    let target = EvidenceTarget {
        kind: Some(EvidenceKind::Text),
        chat: Some(chat),
        id,
        hash: Some(hash.as_str()),
    };
    assert_eq!(
        body_of(&family, target).await,
        Err(EvidenceRefusal::NotFound)
    );
}
