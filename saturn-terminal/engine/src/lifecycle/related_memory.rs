//! 관련 원문 선주입 테스트(#600): 새 session의 첫 작업 입력 앞에 같은 채팅의 관련 원문을 미리 넣는 실험 옵션 `context.select.related`가
//! 기본으로 꺼져 있고, 켜면 종류·채팅·번호·해시 참조와 원문이 실제 provider 호출과 전달 패킷 기록에 같게 남으며, 못 넣은 이유가 기록되는지 확인한다.
//! 설계: docs/design/context-management.md#단계별-기억-확장, docs/design/context-selection.md#보존-우선-선별-계약

use saturn_core::queue::QueuedInput;
use saturn_core::sessions::evidence::text_hash;
use saturn_protocol::ids::{ChatId, LedgerSeq, SessionId};
use saturn_protocol::rpc::EvidenceKind;
use saturn_protocol::state::InputState;

use super::constraint_handoff::packet_of;
use super::support::{Flow, idle_reply, text, tool_read, tool_result, turn_completed};
use crate::dispatch::Start;
use crate::handoff::{RelatedLog, tool_records};
use crate::providers::test_support::{CLAUDE, CODEX};
use crate::store::{PacketItemRow, PacketState, StoredPacket, sha256_hex};
use crate::switch::OpenPlan;

const CLAUDE_FIRST: SessionId = SessionId(2);

const RANK: &str = "[context.select]\nrelated = \"rank\"\n";

/// `RANK`에 더해 Claude의 `P_max`를 400토큰(1,600자)으로 줄인다. 창은 그대로라 전송 가능 상한은 넉넉하다.
const RANK_TIGHT: &str =
    "[context.select]\nrelated = \"rank\"\n[provider.claude.context]\nt_abs = 4000\n";

/// `RANK`에 더해 `vault` 아래 파일 읽기를 거부한다.
const RANK_NO_VAULT: &str =
    "[context.select]\nrelated = \"rank\"\n[permission.read]\n\"*/vault/*\" = \"deny\"\n";

async fn flow_with(config: &str) -> Flow {
    let replies = (0..12).map(|_| idle_reply(0.95)).collect();
    let mut flow = Flow::with_config(config, replies).await;
    flow.add_provider(CODEX);
    flow.engine.switch_provider(flow.chat, CODEX);
    flow
}

/// Codex가 입력 하나에 `answer`로 답하고 `path`를 읽은 결과가 `body`인 턴을 끝낸다.
async fn turn_with(flow: &mut Flow, input: &str, (call, path, body): (&str, &str, &str)) {
    flow.submit(input).await;
    let agent = flow.agent();
    flow.event(CODEX, text(agent, &format!("done {call}")))
        .await;
    flow.event(CODEX, tool_read(agent, call, path)).await;
    flow.event(CODEX, tool_result(agent, call, body)).await;
    flow.event(CODEX, turn_completed(agent)).await;
}

/// 도구 호출 없이 입력과 답만 있는 턴.
async fn talk(flow: &mut Flow, input: &str, answer: &str) {
    flow.submit(input).await;
    let agent = flow.agent();
    flow.event(CODEX, text(agent, answer)).await;
    flow.event(CODEX, turn_completed(agent)).await;
}

/// 채팅의 도구 기록 `(기록 번호, 패킷이 넣는 원문)`. 패킷이 쓰는 후보와 같은 재료다.
async fn tool_texts(flow: &Flow) -> Vec<(u64, String)> {
    let rows = flow
        .engine
        .store
        .ledger_since(flow.chat, LedgerSeq(0))
        .await
        .unwrap();
    tool_records(&rows)
        .into_iter()
        .map(|record| (record.seq.0, record.text))
        .collect()
}

async fn tail_of(flow: &Flow) -> u64 {
    flow.engine
        .store
        .ledger_since(flow.chat, LedgerSeq(0))
        .await
        .unwrap()
        .last()
        .map(|row| row.seq.0)
        .expect("the chat should have records")
}

async fn stored_claude(flow: &Flow) -> StoredPacket {
    let stored: Vec<StoredPacket> = flow
        .engine
        .store
        .packets_of_chat(flow.chat)
        .await
        .unwrap()
        .into_iter()
        .filter(|stored| stored.session == CLAUDE_FIRST)
        .collect();
    let [stored] = &stored[..] else {
        panic!("one packet attempt should be recorded: {stored:?}");
    };
    stored.clone()
}

type RelatedRow = (String, u64, Option<String>, Option<String>, Option<String>);

async fn related_rows(flow: &Flow, stored: &StoredPacket) -> Vec<RelatedRow> {
    flow.engine.store.packet_related(stored.id).await.unwrap()
}

/// 보내기 직전 계획을 세운다. 입력은 접수만 하고 보내지 않아 계획과 적용 사이에 기록과 권한을 바꿀 수 있다.
async fn planned(flow: &mut Flow, input: &str) -> (QueuedInput, OpenPlan) {
    flow.engine.switch_provider(flow.chat, CLAUDE);
    let id = flow.accept_only(input).await;
    let record = flow.record(id);
    let agent = flow.agent();
    let plan = flow
        .engine
        .plan_open(&record, Start::Turn(agent))
        .await
        .expect("the switch should be planned");
    (record, plan)
}

fn related_of(rows: &[PacketItemRow]) -> Vec<(&str, u64, Option<&str>, Option<&'static str>)> {
    rows.iter()
        .filter(|row| row.zone.starts_with("Related"))
        .map(|row| {
            (
                row.zone.as_str(),
                row.ref_id,
                row.form.as_deref(),
                row.reason,
            )
        })
        .collect()
}

fn reference(chat: ChatId, id: u64, body: &str) -> String {
    format!("[tool:{}:{}:{}]\n{body}", chat.0, id, text_hash(body))
}

// #600: 현재 입력은 접수 뒤에도 실행 기록에 없고, 같은 글자의 과거 입력만 후보에 남는다
#[tokio::test]
async fn an_accepted_input_is_not_its_own_related_candidate() {
    let mut flow = flow_with(RANK).await;
    talk(&mut flow, "repeat this task", "finished before").await;
    let before = tail_of(&flow).await;
    flow.engine.switch_provider(flow.chat, CLAUDE);
    let current = flow.accept_only("repeat this task").await;
    let record = flow.record(current);
    let agent = flow.agent();

    let plan = flow
        .engine
        .plan_open(&record, Start::Turn(agent))
        .await
        .expect("the switch should be planned");
    let search = flow
        .engine
        .related_search(flow.chat, &record.text)
        .await
        .unwrap();
    let inputs: Vec<_> = search
        .ranked
        .iter()
        .filter(|found| found.kind == EvidenceKind::Input)
        .collect();

    assert!(plan.packet_text().is_some());
    assert_eq!(search.tail.0, before);
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].text, record.text);
}

// #600: 설정을 켜지 않으면 새 session 패킷에 관련 원문 구역도 기록 행도 없다
#[tokio::test]
async fn related_records_are_not_applied_by_default() {
    let mut flow = flow_with("").await;
    let claude = flow.fake.clone();
    turn_with(
        &mut flow,
        "inspect the cache",
        ("c1", "src/cache.rs", "cache eviction uses LRU"),
    )
    .await;
    flow.engine.switch_provider(flow.chat, CLAUDE);

    let sent = flow.submit("inspect the cache again").await;

    assert_eq!(flow.state(sent), InputState::Applied);
    let packet = packet_of(&claude);
    assert!(!packet.contains("Related original records"), "{packet}");
    let stored = stored_claude(&flow).await;
    assert!(related_rows(&flow, &stored).await.is_empty());
}

// #600: 켜면 관련 도구 원문이 `종류:채팅:번호:해시` 참조와 함께 실제 첫 입력에 한 번만 실리고, 같은 참조와 해시가 패킷 기록에 남는다.
// 보호한 대화 본문은 종류 있는 번호로 걸러져 관련 원문으로 다시 실리지 않는다
#[tokio::test]
async fn related_tool_originals_are_prepended_with_reference_hash_and_no_duplicates() {
    let mut flow = flow_with(RANK).await;
    let claude = flow.fake.clone();
    turn_with(
        &mut flow,
        "inspect the cache",
        ("c1", "src/cache.rs", "cache eviction uses LRU"),
    )
    .await;
    turn_with(
        &mut flow,
        "inspect the parser",
        ("c2", "src/parser.rs", "parser handles utf8 tokens"),
    )
    .await;
    flow.engine.switch_provider(flow.chat, CLAUDE);

    // 입력 본문이 이전 입력과 같은 글이라도 대화 본문은 종류 있는 번호로 구분되어 한 번만 실린다
    let sent = flow.submit("inspect the cache").await;

    assert_eq!(flow.state(sent), InputState::Applied);
    let packet = packet_of(&claude);
    let tools = tool_texts(&flow).await;
    assert_eq!(tools.len(), 2);
    for (id, body) in &tools {
        assert_eq!(
            packet.matches(&reference(flow.chat, *id, body)).count(),
            1,
            "{packet}"
        );
        assert_eq!(packet.matches(body.as_str()).count(), 1, "{packet}");
    }
    assert_eq!(packet.matches("User: inspect the cache").count(), 1);
    assert_eq!(packet.matches("User: inspect the parser").count(), 1);

    let stored = stored_claude(&flow).await;
    assert_eq!(stored.state, PacketState::Sent);
    assert_eq!(stored.input, Some(sent));
    assert_eq!(stored.body_hash, sha256_hex(packet.as_bytes()));
    let chat_revision: i64 =
        sqlx::query_scalar("SELECT chat_revision FROM handoff_packets WHERE id = ?")
            .bind(i64::try_from(stored.id.0).unwrap())
            .fetch_one(flow.engine.store.pool())
            .await
            .unwrap();
    assert_eq!(u64::try_from(chat_revision).unwrap(), tail_of(&flow).await);
    let mut rows = related_rows(&flow, &stored).await;
    rows.sort_by_key(|row| row.1);
    let expected: Vec<RelatedRow> = tools
        .iter()
        .map(|(id, body)| {
            (
                "Related-tool".to_owned(),
                *id,
                Some("Full".to_owned()),
                None,
                Some(text_hash(body)),
            )
        })
        .collect();
    assert_eq!(rows, expected);
}

// #600: 후보가 없으면(대화 본문은 모두 보호 중이라 후보가 아니다) 구역 없이 보내고 이유를 기록한다
#[tokio::test]
async fn no_candidates_sends_without_a_section_and_records_the_reason() {
    let mut flow = flow_with(RANK).await;
    let claude = flow.fake.clone();
    talk(&mut flow, "first question", "first answer").await;
    flow.engine.switch_provider(flow.chat, CLAUDE);

    let sent = flow.submit("second question").await;

    assert_eq!(flow.state(sent), InputState::Applied);
    let packet = packet_of(&claude);
    assert!(!packet.contains("Related original records"), "{packet}");
    assert!(packet.contains("User: first question"), "{packet}");
    let stored = stored_claude(&flow).await;
    let tail = tail_of(&flow).await;
    assert_eq!(
        related_rows(&flow, &stored).await,
        vec![(
            "Related".to_owned(),
            tail,
            None,
            Some("no_candidates".to_owned()),
            None
        )]
    );
}

// #600: 보호 본문이 목표 예산을 넘으면 관련 원문은 하나도 줄여 넣지 않고 기록마다 `budget`으로 뺀다. 대화 본문은 그대로 실린다
#[tokio::test]
async fn related_records_over_the_budget_are_left_out_whole_with_a_reason() {
    let mut flow = flow_with(RANK_TIGHT).await;
    let claude = flow.fake.clone();
    let answers: Vec<String> = ["a", "b", "c"]
        .iter()
        .map(|mark| format!("{mark}{}", ".".repeat(599)))
        .collect();
    for (number, answer) in answers.iter().enumerate() {
        flow.submit(&format!("step {number}")).await;
        let agent = flow.agent();
        flow.event(CODEX, text(agent, answer)).await;
        flow.event(
            CODEX,
            tool_read(agent, &format!("c{number}"), "src/cache.rs"),
        )
        .await;
        flow.event(CODEX, tool_result(agent, &format!("c{number}"), "TOOLBODY"))
            .await;
        flow.event(CODEX, turn_completed(agent)).await;
    }
    flow.engine.switch_provider(flow.chat, CLAUDE);

    let sent = flow.submit("continue").await;

    assert_eq!(flow.state(sent), InputState::Applied);
    let packet = packet_of(&claude);
    for answer in &answers {
        assert!(packet.contains(answer.as_str()), "{packet}");
    }
    assert!(!packet.contains("TOOLBODY"), "{packet}");
    assert!(!packet.contains("Related original records"), "{packet}");
    let stored = stored_claude(&flow).await;
    let mut rows = related_rows(&flow, &stored).await;
    rows.sort_by_key(|row| row.1);
    let expected: Vec<RelatedRow> = tool_texts(&flow)
        .await
        .iter()
        .map(|(id, body)| {
            (
                "Related-tool".to_owned(),
                *id,
                None,
                Some("budget".to_owned()),
                Some(text_hash(body)),
            )
        })
        .collect();
    assert_eq!(expected.len(), 3);
    assert_eq!(rows, expected);
}

// #600: 검색이 실패하면 아무것도 넣지 않고 입력은 막지 않으며 `retrieval_failed`로 남긴다
#[tokio::test]
async fn a_failed_retrieval_adds_nothing_and_is_recorded() {
    let mut flow = flow_with(RANK).await;
    talk(&mut flow, "first question", "first answer").await;
    flow.engine.switch_provider(flow.chat, CLAUDE);
    let id = flow.accept_only("second question").await;
    let mut record = flow.record(id);
    record.chat = ChatId(9_999);
    let settings = flow
        .engine
        .settings
        .at(&flow.engine.store, record.settings)
        .await
        .unwrap();
    let budget = settings.context_budget(CLAUDE, flow.engine.registry.context_defaults(CLAUDE));

    let (source, log) = flow
        .engine
        .attach_related((&record, &settings, &budget), None)
        .await;

    assert!(source.is_none());
    assert_eq!(
        log,
        Some(RelatedLog::none(LedgerSeq(0), "retrieval_failed"))
    );
}

/// 도구 기록 둘이 있는 채팅에서 첫 도구가 `path_of_first`를 읽는다. 보내기 직전 계획과 그 계획의 도구 기록 번호를 돌려준다.
async fn planned_with_two_tools(
    flow: &mut Flow,
    path_of_first: &str,
) -> (QueuedInput, OpenPlan, Vec<(u64, String)>) {
    turn_with(
        flow,
        "inspect the vault",
        ("c1", path_of_first, "vault key listing"),
    )
    .await;
    turn_with(
        flow,
        "inspect the parser",
        ("c2", "src/parser.rs", "parser handles utf8 tokens"),
    )
    .await;
    let (record, plan) = planned(flow, "vault key listing").await;
    let tools = tool_texts(flow).await;
    let (rows, _) = plan.packet_rows();
    assert_eq!(
        related_of(&rows).len(),
        2,
        "both tool records should be selected before the recheck: {rows:?}"
    );
    (record, plan, tools)
}

// #600: 적용 직전에 채팅 revision이 달라졌으면 늦은 결과라 관련 원문을 모두 버리고 `stale_revision`으로 기록한다. 대화 본문은 그대로다
#[tokio::test]
async fn a_moved_chat_revision_discards_the_selection_before_it_is_applied() {
    let mut flow = flow_with(RANK).await;
    let (record, plan, tools) = planned_with_two_tools(&mut flow, "src/auth.rs").await;
    let rows = flow
        .engine
        .store
        .ledger_since(flow.chat, LedgerSeq(0))
        .await
        .unwrap();
    let last = rows.last().expect("the chat should have records");
    flow.engine
        .store
        .append_event(last.run, flow.chat, &text(flow.agent(), "a late record"))
        .await
        .unwrap();

    let plan = flow.engine.recheck_related(&record, plan).await;

    let (rows, carries) = plan.packet_rows();
    assert!(
        carries,
        "the rebuilt packet should still carry the dialogue"
    );
    let mut reasons: Vec<_> = related_of(&rows)
        .into_iter()
        .map(|(zone, id, form, reason)| (zone.to_owned(), id, form.is_some(), reason))
        .collect();
    reasons.sort();
    let expected: Vec<_> = tools
        .iter()
        .map(|(id, _)| {
            (
                "Related-tool".to_owned(),
                *id,
                false,
                Some("stale_revision"),
            )
        })
        .collect();
    assert_eq!(reasons, expected);
    let packet = plan.packet_text().expect("the packet should still be sent");
    assert!(!packet.contains("Related original records"), "{packet}");
    assert!(packet.contains("User: inspect the vault"), "{packet}");
}

// #600: 읽기 권한을 잃은 기록만 `scope`로 빼고 나머지는 그대로 싣는다. 오래된 원문으로 대신하지 않는다
#[tokio::test]
async fn a_revoked_read_permission_drops_only_that_record_with_a_reason() {
    let mut flow = flow_with(RANK).await;
    let (record, plan, tools) = planned_with_two_tools(&mut flow, "vault/keys.txt").await;
    flow.fixture.write_user_config(RANK_NO_VAULT);
    flow.engine.watch_settings().await;
    flow.engine.watch_settings().await;

    let plan = flow.engine.recheck_related(&record, plan).await;

    let (rows, carries) = plan.packet_rows();
    assert!(carries);
    let (vault, parser) = (&tools[0], &tools[1]);
    let mut got: Vec<_> = related_of(&rows)
        .into_iter()
        .map(|(_, id, form, reason)| (id, form.is_some(), reason))
        .collect();
    got.sort();
    assert_eq!(
        got,
        vec![(vault.0, false, Some("scope")), (parser.0, true, None)]
    );
    let packet = plan.packet_text().expect("the packet should still be sent");
    assert!(!packet.contains(vault.1.as_str()), "{packet}");
    assert!(
        packet.contains(&reference(flow.chat, parser.0, &parser.1)),
        "{packet}"
    );
}

// #600: 원문이 바뀐 기록은 `changed`로 빼고 바뀐 내용을 싣지 않는다
#[tokio::test]
async fn a_changed_source_is_dropped_with_a_reason_and_never_substituted() {
    let mut flow = flow_with(RANK).await;
    let (record, plan, tools) = planned_with_two_tools(&mut flow, "src/auth.rs").await;
    sqlx::query("UPDATE events SET body = replace(body, 'vault key listing', 'vault key rewrite') WHERE chat_id = ?")
        .bind(i64::try_from(flow.chat.0).unwrap())
        .execute(flow.engine.store.pool())
        .await
        .unwrap();

    let plan = flow.engine.recheck_related(&record, plan).await;

    let (rows, carries) = plan.packet_rows();
    assert!(carries);
    let mut got: Vec<_> = related_of(&rows)
        .into_iter()
        .map(|(_, id, form, reason)| (id, form.is_some(), reason))
        .collect();
    got.sort();
    assert_eq!(
        got,
        vec![
            (tools[0].0, false, Some("changed")),
            (tools[1].0, true, None)
        ]
    );
    let packet = plan.packet_text().expect("the packet should still be sent");
    assert!(!packet.contains("vault key listing"), "{packet}");
    assert!(!packet.contains("vault key rewrite"), "{packet}");
}

// #600: 기록에서 사라진 기록은 `deleted`로 뺀다
#[tokio::test]
async fn a_deleted_source_is_dropped_with_a_reason() {
    let mut flow = flow_with(RANK).await;
    let (record, plan, tools) = planned_with_two_tools(&mut flow, "src/auth.rs").await;
    sqlx::query("DELETE FROM events WHERE chat_id = ? AND seq = ?")
        .bind(i64::try_from(flow.chat.0).unwrap())
        .bind(i64::try_from(tools[0].0).unwrap())
        .execute(flow.engine.store.pool())
        .await
        .unwrap();

    let plan = flow.engine.recheck_related(&record, plan).await;

    let (rows, carries) = plan.packet_rows();
    assert!(carries);
    let mut got: Vec<_> = related_of(&rows)
        .into_iter()
        .map(|(_, id, form, reason)| (id, form.is_some(), reason))
        .collect();
    got.sort();
    assert_eq!(
        got,
        vec![
            (tools[0].0, false, Some("deleted")),
            (tools[1].0, true, None)
        ]
    );
    let packet = plan.packet_text().expect("the packet should still be sent");
    assert!(!packet.contains("vault key listing"), "{packet}");
}
