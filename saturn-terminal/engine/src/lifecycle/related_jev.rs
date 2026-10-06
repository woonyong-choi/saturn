//! 관련 원문 Jev 실험(`context.select.related = jev`) 테스트: 종류 있는 후보 스냅샷과 요청 안 임시 번호로만 답을 해석하고,
//! 접수 입력·설정·기록 끝·후보·대상 중 하나라도 달라진 답, 늦은 답, 실패한 답, 승격할 후보가 없는 답은 `rank`로 고정하며,
//! 같은 전문 예산과 기록 규칙을 쓰는지 확인한다.
//! 설계: docs/design/context-selection.md#관련-원문-jev-실험-계약

use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime};

use saturn_core::queue::QueuedInput;
use saturn_core::routers::{Answer, RouterError, RouterResponse};
use saturn_core::sessions::{AgentRole, SendTarget};
use saturn_protocol::ids::SettingsRevision;
use saturn_protocol::rpc::EvidenceKind;
use serde_json::json;

use super::constraint_handoff::packet_of;
use super::related_memory::{
    CLAUDE_FIRST, RelatedRow, reference, related_rows, stored_claude, tool_texts, turn_with,
};
use super::support::{Flow, idle_reply, text, tool_read, tool_result, turn_completed};
use super::{FakeReply, ok, status};
use crate::evidence::RelatedFound;
use crate::packet_select::{CompactReply, Trigger};
use crate::providers::test_support::{CLAUDE, CODEX};
use crate::related::{RelatedOrder, RelatedSnapshot};
use crate::routers::RouterExchange;
use saturn_core::sessions::evidence::text_hash;

const JEV: &str = "[context.select]\nrelated = \"jev\"\n";
const RANK: &str = "[context.select]\nrelated = \"rank\"\n";
const JEV_TIGHT: &str =
    "[context.select]\nrelated = \"jev\"\n[provider.claude.context]\nt_abs = 4000\n";
const RANK_TIGHT: &str =
    "[context.select]\nrelated = \"rank\"\n[provider.claude.context]\nt_abs = 4000\n";
const BOTH: &str = "[context.select]\nrelated = \"jev\"\npacket = \"jev\"\n";

/// 입력 판단 답을 `inputs`개만 채운 흐름. 그 뒤에 `push_reply`로 넣는 답이 판단 호출의 답이다.
async fn flow_with(config: &str, inputs: usize) -> Flow {
    let replies = (0..inputs).map(|_| idle_reply(0.95)).collect();
    let mut flow = Flow::with_config(config, replies).await;
    flow.add_provider(CODEX);
    flow.engine.switch_provider(flow.chat, CODEX);
    flow
}

/// `candidate_<번호>_direct`에 P(yes)를 주는 router 답.
fn related_reply(probabilities: &[(u32, f64)]) -> FakeReply {
    let mut answers = serde_json::Map::new();
    for (ordinal, probability) in probabilities {
        answers.insert(
            format!("candidate_{ordinal}_direct"),
            json!({ "type": "noul", "noul": probability }),
        );
    }
    ok(&json!({
        "model": "jev-1.13.0",
        "answers": answers,
        "usage": { "input_tokens": 40, "output_tokens": 8 },
    })
    .to_string())
}

/// 도구 기록 둘(`cache`, `parser`)이 있는 채팅을 Claude로 바꾸기 직전까지 만든다.
async fn two_tools(config: &str) -> Flow {
    let mut flow = flow_with(config, 3).await;
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
    flow
}

/// 같은 입력 본문으로 검색한 도구 원문 후보. 순위 순서이고 `jev`가 묻는 순서와 같다.
async fn tool_candidates(flow: &Flow, query: &str) -> Vec<RelatedFound> {
    flow.engine
        .related_search(flow.chat, query)
        .await
        .unwrap()
        .ranked
        .into_iter()
        .filter(|found| found.kind == EvidenceKind::Tool)
        .collect()
}

/// `(구역, 번호, 선택 방식, 이유)`. 관련 원문 행만 기록 순서로 본다.
async fn selector_rows(
    flow: &Flow,
    stored: &crate::store::StoredPacket,
) -> Vec<(String, u64, String, Option<String>)> {
    use sqlx::Row;
    sqlx::query(
        "SELECT zone, ref_id, selector, reason FROM handoff_packet_items \
         WHERE packet_id = ? AND zone LIKE 'Related%' ORDER BY rowid",
    )
    .bind(i64::try_from(stored.id.0).unwrap())
    .fetch_all(flow.engine.store.pool())
    .await
    .unwrap()
    .iter()
    .map(|row| {
        (
            row.get("zone"),
            u64::try_from(row.get::<i64, _>("ref_id")).unwrap(),
            row.get("selector"),
            row.get("reason"),
        )
    })
    .collect()
}

// cost: time O(j), heap O(j), stack O(1), io 2
// vars: j = 판단 기록 파일 바이트 수
// basis: estimate
async fn judgments(flow: &Flow) -> String {
    let path = flow.fixture.root.path().join("judgments.jsonl");
    flow.engine.store.export_judgments(&path).await.unwrap();
    std::fs::read_to_string(path).unwrap()
}

// #600: 확신 있는 긍정이 앞으로 올라 종류 있는 참조와 함께 먼저 실리고, 나머지는 기존 순위 순으로 뒤에 실린다. 요청은 임시 번호와 종류 있는 참조만 쓰고
// 도구 번호 전용 `compact` 질문 번호를 쓰지 않는다
#[tokio::test]
async fn judged_candidates_are_applied_by_typed_reference_with_the_requested_and_applied_selector()
{
    let mut flow = two_tools(JEV).await;
    let claude = flow.fake.clone();
    let query = "inspect the cache again";
    let candidates = tool_candidates(&flow, query).await;
    assert_eq!(candidates.len(), 2);
    flow.transport
        .push_reply(related_reply(&[(1, 0.05), (2, 0.95)]));
    let calls = flow.router_calls();

    flow.submit(query).await;

    assert_eq!(flow.router_calls() - calls, 2);
    let request = flow.transport.calls().last().unwrap().2.clone().unwrap();
    assert!(request.contains("candidate_1_direct"), "{request}");
    assert!(request.contains("candidate_2_direct"), "{request}");
    assert!(!request.contains("call_"), "{request}");
    assert!(!request.contains("result_"), "{request}");
    // 요청 안의 순서가 후보 순서이고 후보는 종류, 채팅, 번호, 해시로 구분된다
    let refs: Vec<usize> = candidates
        .iter()
        .map(|found| {
            let label = format!("[tool:{}:{}:{}]", flow.chat.0, found.id, found.hash);
            request
                .find(&label)
                .expect("the typed reference should be asked")
        })
        .collect();
    assert!(refs[0] < refs[1], "{request}");
    let kept = &candidates[1];
    let dropped = &candidates[0];
    let packet = packet_of(&claude);
    assert_eq!(
        packet
            .matches(&reference(flow.chat, kept.id, &kept.text))
            .count(),
        1,
        "{packet}"
    );
    // 아무 후보도 빠지지 않고 긍정이 먼저 놓인다
    assert_eq!(
        packet
            .matches(&reference(flow.chat, dropped.id, &dropped.text))
            .count(),
        1,
        "{packet}"
    );
    assert!(
        packet.find(&reference(flow.chat, kept.id, &kept.text))
            < packet.find(&reference(flow.chat, dropped.id, &dropped.text)),
        "{packet}"
    );
    let stored = stored_claude(&flow).await;
    let rows = selector_rows(&flow, &stored).await;
    assert_eq!(
        rows,
        vec![
            (
                "Related-tool".to_owned(),
                kept.id,
                "related_jev".to_owned(),
                None
            ),
            (
                "Related-tool".to_owned(),
                dropped.id,
                "related_jev".to_owned(),
                None
            ),
        ]
    );
    let lines = judgments(&flow).await;
    assert_eq!(lines.matches("related@2.0").count(), 1, "{lines}");
}

// #600: 확신 있는 긍정이 없으면 승격하지 않고 `rank`로 고정하며 `jev_no_promotion`을 남긴다. 아무 후보도 제외하지 않는다
#[tokio::test]
async fn without_a_confident_positive_nothing_is_promoted_and_rank_is_applied() {
    let mut flow = two_tools(JEV).await;
    let claude = flow.fake.clone();
    flow.transport
        .push_reply(related_reply(&[(1, 0.02), (2, 0.1)]));

    flow.submit("inspect the cache again").await;

    let packet = packet_of(&claude);
    assert!(packet.contains("Related original records"), "{packet}");
    let stored = stored_claude(&flow).await;
    let rows = selector_rows(&flow, &stored).await;
    assert_eq!(rows.iter().filter(|row| row.0 == "Related-tool").count(), 2);
    assert!(
        rows.iter()
            .filter(|row| row.0 == "Related-tool")
            .all(|row| row.2 == "related_rank" && row.3.is_none()),
        "{rows:?}"
    );
    let note: Vec<_> = rows.iter().filter(|row| row.0 == "Related").collect();
    assert_eq!(note.len(), 1, "{rows:?}");
    assert_eq!(note[0].2, "related_jev");
    assert_eq!(note[0].3.as_deref(), Some("jev_no_promotion"));
}

// #600: 늦은 답, router 실패, 승격할 후보가 없는 답, 형식이 틀린 확률은 `rank`로 고정되고 같은 원문을 싣는다. 요청 방식과 실제 방식과 이유가 남는다
#[tokio::test]
async fn unusable_answers_fall_back_to_rank_and_log_requested_and_applied() {
    let mut rank = two_tools(RANK).await;
    let rank_claude = rank.fake.clone();
    rank.submit("inspect the cache again").await;
    let expected = rank_packet_bodies(&packet_of(&rank_claude), &rank).await;
    assert_eq!(expected.len(), 2);

    let cases: Vec<(&str, FakeReply, &str)> = vec![
        ("late", related_reply(&[(1, 0.05), (2, 0.95)]), "jev_late"),
        (
            "router failure",
            status(401, "{}", Vec::new()),
            "jev_failed",
        ),
        (
            "no confident positive",
            related_reply(&[(1, 0.45), (2, 0.75)]),
            "jev_no_promotion",
        ),
        (
            "order already matches rank",
            related_reply(&[(1, 0.95), (2, 0.1)]),
            "jev_no_promotion",
        ),
        ("missing answer", related_reply(&[(2, 0.9)]), "jev_invalid"),
        (
            "probability the router rejects",
            related_reply(&[(1, 1.4), (2, 0.9)]),
            "jev_failed",
        ),
        (
            "tool call numbers are not ordinals",
            ok(&json!({
                "model": "jev-1.13.0",
                "answers": {
                    "call_1_keep": { "type": "noul", "noul": 0.9 },
                    "result_1_keep": { "type": "noul", "noul": 0.9 },
                },
                "usage": { "input_tokens": 1, "output_tokens": 1 },
            })
            .to_string()),
            "jev_invalid",
        ),
    ];
    for (name, reply, reason) in cases {
        let mut flow = two_tools(JEV).await;
        let claude = flow.fake.clone();
        if name == "late" {
            // 기다림(시험에서는 0.3초)보다 늦게 온 답이다
            flow.transport
                .push_reply_after(reply, Duration::from_secs(3));
        } else {
            flow.transport.push_reply(reply);
        }

        flow.submit("inspect the cache again").await;

        let got = rank_packet_bodies(&packet_of(&claude), &flow).await;
        assert_eq!(got, expected, "{name}");
        let stored = stored_claude(&flow).await;
        let rows = selector_rows(&flow, &stored).await;
        let applied: Vec<_> = rows.iter().filter(|row| row.0 == "Related-tool").collect();
        assert_eq!(applied.len(), 2, "{name}");
        assert!(
            applied
                .iter()
                .all(|row| row.2 == "related_rank" && row.3.is_none()),
            "{name}: {rows:?}"
        );
        let note: Vec<_> = rows.iter().filter(|row| row.0 == "Related").collect();
        assert_eq!(note.len(), 1, "{name}: {rows:?}");
        assert_eq!(note[0].2, "related_jev", "{name}");
        assert_eq!(note[0].3.as_deref(), Some(reason), "{name}");
        let lines = judgments(&flow).await;
        assert_eq!(lines.matches("related@2.0").count(), 1, "{name}: {lines}");
        if name == "late" {
            assert!(lines.contains("late"), "{lines}");
        }
    }
}

/// 보낸 패킷에 실린 도구 원문 참조 목록.
async fn rank_packet_bodies(packet: &str, flow: &Flow) -> Vec<String> {
    tool_texts(flow)
        .await
        .into_iter()
        .filter(|(id, body)| packet.contains(&reference(flow.chat, *id, body)))
        .map(|(id, _)| id.to_string())
        .collect()
}

// #600: 요청 한도 때문에 원문 일부만 볼 수 있으면 질문에 관측 범위를 밝히고 판단 기록에 `clipped`를 남긴다. 전체를 판단했다고 주장하지 않는다
#[tokio::test]
async fn a_clipped_candidate_states_the_observed_range_and_is_recorded() {
    let mut flow = flow_with(JEV, 2).await;
    turn_with(
        &mut flow,
        "inspect the cache",
        ("c1", "src/cache.rs", &"z".repeat(9_000)),
    )
    .await;
    flow.engine.switch_provider(flow.chat, CLAUDE);
    flow.transport.push_reply(related_reply(&[(1, 0.9)]));

    flow.submit("inspect the cache again").await;

    let request = flow.transport.calls().last().unwrap().2.clone().unwrap();
    assert!(
        request.contains("of 9022 characters, 8682 omitted"),
        "{request}"
    );
    // 앞 120자와 뒤 220자만 실린다
    // 도구 이름 줄이 앞 120자 안에 든다
    assert!(
        (300..=340).contains(&request.matches('z').count()),
        "{request}"
    );
    let lines = judgments(&flow).await;
    assert!(lines.contains("clipped"), "{lines}");
}

/// 같은 세 도구 결과를 보내는 채팅의 패킷과 관련 원문 행. 첫 후보부터 확률을 내림차순으로 줘 두 방식의 후보 순서가 같다.
async fn three_tools(config: &str) -> (String, Vec<RelatedRow>) {
    let mut flow = flow_with(config, 4).await;
    let claude = flow.fake.clone();
    for number in 0..3 {
        flow.submit(&format!("step {number}")).await;
        let agent = flow.agent();
        flow.event(CODEX, text(agent, &format!("{number}{}", ".".repeat(599))))
            .await;
        let call = format!("c{number}");
        flow.event(CODEX, tool_read(agent, &call, "src/cache.rs"))
            .await;
        flow.event(
            CODEX,
            tool_result(
                agent,
                &call,
                &format!("TOOLBODY{number} {}", "y".repeat(200)),
            ),
        )
        .await;
        flow.event(CODEX, turn_completed(agent)).await;
    }
    flow.engine.switch_provider(flow.chat, CLAUDE);
    if config.contains("jev") {
        flow.transport
            .push_reply(related_reply(&[(1, 0.95), (2, 0.9), (3, 0.85)]));
    }
    flow.submit("continue").await;
    let stored = stored_claude(&flow).await;
    let mut rows = related_rows(&flow, &stored).await;
    rows.sort_by_key(|row| row.1);
    assert_eq!(stored.session, CLAUDE_FIRST);
    (packet_of(&claude), rows)
}

// #600: 같은 후보에 같은 전문 예산이므로 `jev`가 순서를 바꾸지 않으면 `rank`와 같은 원문을 싣고 같은 `budget`으로 뺀다
#[tokio::test]
async fn jev_and_rank_share_the_same_budget_and_omission_reasons() {
    for (rank_config, jev_config) in [(RANK_TIGHT, JEV_TIGHT), (RANK, JEV)] {
        let (rank_packet, rank_rows) = three_tools(rank_config).await;
        let (jev_packet, mut jev_rows) = three_tools(jev_config).await;
        // 순서가 그대로라 `rank`로 고정되고, 요청 방식과 이유를 적은 구역 `Related` 행만 더 있다
        jev_rows.retain(|row| row.0 != "Related");

        assert_eq!(jev_rows, rank_rows, "{jev_config}");
        assert_eq!(rank_rows.len(), 3);
        for number in 0..3 {
            let mark = format!("TOOLBODY{number} ");
            assert_eq!(
                jev_packet.contains(&mark),
                rank_packet.contains(&mark),
                "{jev_config}: {mark}"
            );
        }
    }
}

// #600: 패킷 `compact`와 관련 원문 Jev를 함께 켜도 답 자리가 겹치지 않고 각자 한 번씩 묻고 한 번씩 기록한다
#[tokio::test]
async fn compact_and_related_judgments_do_not_share_a_reply_slot() {
    let mut flow = two_tools(BOTH).await;
    let claude = flow.fake.clone();
    // 앞의 `compact` 호출은 답 없음으로 순위 순서로 돌아가고, 뒤의 `related` 호출만 후보를 판단한다
    flow.transport.push_reply(ok(&json!({
        "model": "jev-1.13.0",
        "answers": {},
        "usage": { "input_tokens": 1, "output_tokens": 1 },
    })
    .to_string()));
    flow.transport
        .push_reply(related_reply(&[(1, 0.1), (2, 0.9)]));
    let calls = flow.router_calls();

    flow.submit("inspect the cache again").await;

    assert_eq!(flow.router_calls() - calls, 3);
    let _ = packet_of(&claude);
    let stored = stored_claude(&flow).await;
    let rows = selector_rows(&flow, &stored).await;
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert!(rows.iter().all(|row| row.2 == "related_jev"), "{rows:?}");
    assert!(rows.iter().all(|row| row.3.is_none()), "{rows:?}");
    let lines = judgments(&flow).await;
    assert_eq!(lines.matches("related@2.0").count(), 1, "{lines}");
    assert_eq!(lines.matches("compact@1.0").count(), 1, "{lines}");
}

/// 지금 값에 가하는 변경 하나와 그 이름.
type Change = (&'static str, Box<dyn Fn(&mut RelatedSnapshot)>);

/// 접수한 입력으로 `jev` 스냅샷을 만든다. 검색 결과와 지금 값이 모두 같은 상태다.
struct Asked {
    flow: Flow,
    record: QueuedInput,
    candidates: Vec<RelatedFound>,
    now: RelatedSnapshot,
}

async fn asked() -> Asked {
    let mut flow = two_tools(JEV).await;
    let id = flow.accept_only("inspect the cache again").await;
    let record = flow.record(id);
    let candidates = tool_candidates(&flow, &record.text).await;
    let tail = flow
        .engine
        .related_search(flow.chat, &record.text)
        .await
        .unwrap()
        .tail;
    let settings = flow
        .engine
        .settings
        .at(&flow.engine.store, record.settings)
        .await
        .unwrap();
    let target = SendTarget::New {
        provider: CLAUDE,
        role: AgentRole::Main,
    };
    let now = RelatedSnapshot::of(&record, &settings, tail, (&target, None), &candidates);
    Asked {
        flow,
        record,
        candidates,
        now,
    }
}

impl Asked {
    /// `asked` 스냅샷으로 물은 호출에 `answer`가 돌아와 있다. 답이 `None`이면 기다림 안에 오지 않은 것이다.
    fn reply(
        &mut self,
        asked: &RelatedSnapshot,
        answer: Option<Result<RouterResponse, RouterError>>,
    ) {
        let call = self
            .flow
            .engine
            .related_call(asked, &self.candidates)
            .expect("the request should be built");
        let slot = (self.now.chat, Trigger::Related(self.now.input));
        self.flow.engine.flow.compact_replies.insert(
            slot,
            CompactReply {
                call,
                exchange: answer.map(|result| RouterExchange {
                    sent: String::new(),
                    received: None,
                    result,
                    started_at: SystemTime::now(),
                    elapsed: Duration::from_millis(5),
                    unknown_cost_calls: 0,
                }),
                is_recorded: AtomicBool::new(false),
            },
        );
    }

    async fn gate(&self, now: RelatedSnapshot, candidates: &[RelatedFound]) -> RelatedOrder {
        self.flow
            .engine
            .related_gate(now, candidates)
            .await
            .unwrap_or_else(|_| panic!("a stored answer should not ask again"))
    }
}

fn response(probabilities: &[f64]) -> Result<RouterResponse, RouterError> {
    Ok(RouterResponse {
        model: "jev-1.13.0".to_owned(),
        answers: probabilities
            .iter()
            .zip(1u32..)
            .map(|(probability, ordinal)| {
                (
                    format!("candidate_{ordinal}_direct"),
                    Answer::Noul(*probability),
                )
            })
            .collect(),
        tokens: (10, 2),
    })
}

// #600: 답은 요청한 때의 입력, 설정, 기록 끝, 후보, 새 session 대상이 지금과 모두 같을 때만 쓴다. 하나라도 다르면 `rank`로 고정하고 판단은 `superseded`로 남는다
#[tokio::test]
async fn an_answer_is_used_only_when_every_part_of_the_snapshot_still_matches() {
    let mut fixture = asked().await;
    let asked = fixture.now.clone();
    let changes: Vec<Change> = vec![
        ("input body", Box::new(|now| now.text.push_str(" and more"))),
        (
            "settings revision",
            Box::new(|now| now.settings = SettingsRevision(now.settings.0 + 1)),
        ),
        (
            "related value",
            Box::new(|now| now.related = crate::settings::RelatedSelect::Rank),
        ),
        (
            "ledger tail",
            Box::new(|now| now.tail = saturn_protocol::ids::LedgerSeq(now.tail.0 + 1)),
        ),
        (
            "target model",
            Box::new(|now| now.target.1 = Some("other".to_owned())),
        ),
        (
            "target provider",
            Box::new(|now| {
                now.target.0 = SendTarget::New {
                    provider: CODEX,
                    role: AgentRole::Main,
                }
            }),
        ),
        (
            "candidate hash",
            Box::new(|now| now.refs[0].2 = "0".repeat(64)),
        ),
        (
            "candidate kind with the same number",
            Box::new(|now| now.refs[0].0 = EvidenceKind::Text),
        ),
        ("candidate order", Box::new(|now| now.refs.reverse())),
        (
            "a candidate gone",
            Box::new(|now| {
                now.refs.pop();
            }),
        ),
    ];
    let candidates = fixture.candidates.clone();
    for (name, change) in changes {
        fixture.reply(&asked, Some(response(&[0.9, 0.1])));
        let mut now = asked.clone();
        change(&mut now);

        let order = fixture.gate(now, &candidates).await;

        assert_eq!(order, RelatedOrder::Rank("jev_stale"), "{name}");
    }
    fixture.reply(&asked, Some(response(&[0.1, 0.9])));
    assert_eq!(
        fixture.gate(asked.clone(), &candidates).await,
        RelatedOrder::Judged(vec![1, 0]),
        "an unchanged snapshot applies"
    );
    let lines = judgments(&fixture.flow).await;
    assert!(lines.contains("Superseded"), "{lines}");
}

// #600: 같은 번호의 다른 종류는 서로 다른 후보다. 임시 번호는 후보 순서이고 답은 종류 있는 후보로만 돌아간다
#[tokio::test]
async fn the_same_number_of_different_kinds_stays_two_candidates() {
    let mut fixture = asked().await;
    let tool = RelatedFound {
        kind: EvidenceKind::Tool,
        id: 7,
        hash: text_hash("tool body"),
        text: "tool body".to_owned(),
    };
    let reply = RelatedFound {
        kind: EvidenceKind::Text,
        id: 7,
        hash: text_hash("text body"),
        text: "text body".to_owned(),
    };
    let candidates = vec![tool, reply];
    let settings = fixture
        .flow
        .engine
        .settings
        .at(&fixture.flow.engine.store, fixture.record.settings)
        .await
        .unwrap();
    let snapshot = RelatedSnapshot::of(
        &fixture.record,
        &settings,
        fixture.now.tail,
        (&fixture.now.target.0, None),
        &candidates,
    );
    assert_eq!(snapshot.refs.len(), 2);
    assert_ne!(snapshot.refs[0], snapshot.refs[1]);
    fixture.candidates = candidates.clone();
    fixture.reply(&snapshot, Some(response(&[0.1, 0.9])));
    let call = &fixture.flow.engine.flow.compact_replies
        [&(snapshot.chat, Trigger::Related(snapshot.input))]
        .call;
    let questions = &call.request.sets[0].1;
    assert_eq!(
        questions
            .iter()
            .map(|question| question.id.as_str())
            .collect::<Vec<_>>(),
        ["candidate_1_direct", "candidate_2_direct"]
    );
    assert!(questions[0].text.contains(&format!(
        "[tool:{}:7:{}]",
        snapshot.chat.0,
        text_hash("tool body")
    )));
    assert!(questions[1].text.contains(&format!(
        "[text:{}:7:{}]",
        snapshot.chat.0,
        text_hash("text body")
    )));

    let order = fixture.gate(snapshot, &candidates).await;

    // 둘째 후보(종류가 `text`인 7번)가 앞으로 오고 첫째도 남는다
    assert_eq!(order, RelatedOrder::Judged(vec![1, 0]));
}

// #600: 확신 있는 긍정을 확률 내림차순, 같으면 기존 순위 순으로 올리고 나머지는 순위 순으로 붙인다. 초과 번호의 답은 무시한다
#[tokio::test]
async fn kept_candidates_are_ordered_by_probability_then_rank() {
    let mut fixture = asked().await;
    let asked = fixture.now.clone();
    let cases: [([f64; 2], RelatedOrder); 4] = [
        ([0.8, 0.95], RelatedOrder::Judged(vec![1, 0])),
        ([0.9, 0.9], RelatedOrder::Rank("jev_no_promotion")),
        ([0.1, 0.9], RelatedOrder::Judged(vec![1, 0])),
        ([0.1, 0.05], RelatedOrder::Rank("jev_no_promotion")),
    ];
    let candidates = fixture.candidates.clone();
    for (probabilities, expected) in cases {
        fixture.reply(&asked, Some(response(&probabilities)));
        assert_eq!(
            fixture.gate(asked.clone(), &candidates).await,
            expected,
            "{probabilities:?}"
        );
    }
    // 요청에 없는 셋째 번호의 답은 쓰지 않는다
    let mut extra = response(&[0.9, 0.1]).unwrap();
    extra
        .answers
        .push(("candidate_3_direct".to_owned(), Answer::Noul(0.99)));
    fixture.reply(&asked, Some(Ok(extra)));
    assert_eq!(
        fixture.gate(asked.clone(), &candidates).await,
        RelatedOrder::Rank("jev_no_promotion")
    );
    // 기다림 안에 오지 않은 답은 쓰지 않는다
    fixture.reply(&asked, None);
    assert_eq!(
        fixture.gate(asked, &candidates).await,
        RelatedOrder::Rank("jev_late")
    );
}

// #600: 현재 읽기 권한을 잃은 원문은 후보에서 빠지므로 권한을 잃기 전 후보에 대한 답은 늦은 판단으로 버리고 지금 자료로 `rank`를 다시 만든다
#[tokio::test]
async fn a_revoked_permission_changes_the_candidates_and_discards_the_answer() {
    let mut flow = flow_with(JEV, 3).await;
    turn_with(
        &mut flow,
        "inspect the vault",
        ("c1", "vault/keys.txt", "vault key listing"),
    )
    .await;
    turn_with(
        &mut flow,
        "inspect the parser",
        ("c2", "src/parser.rs", "parser handles utf8 tokens"),
    )
    .await;
    flow.engine.switch_provider(flow.chat, CLAUDE);
    let id = flow.accept_only("inspect the vault again").await;
    let record = flow.record(id);
    let before = tool_candidates(&flow, &record.text).await;
    assert_eq!(before.len(), 2);
    flow.fixture.write_user_config(
        "[context.select]\nrelated = \"jev\"\n[permission.read]\n\"*/vault/*\" = \"deny\"\n",
    );
    flow.engine.watch_settings().await;
    flow.engine.watch_settings().await;
    let after = tool_candidates(&flow, &record.text).await;
    assert_eq!(after.len(), 1);
    let target = SendTarget::New {
        provider: CLAUDE,
        role: AgentRole::Main,
    };
    let settings = flow
        .engine
        .settings
        .at(&flow.engine.store, record.settings)
        .await
        .unwrap();
    let tail = flow
        .engine
        .related_search(flow.chat, &record.text)
        .await
        .unwrap()
        .tail;
    let asked = RelatedSnapshot::of(&record, &settings, tail, (&target, None), &before);
    let now = RelatedSnapshot::of(&record, &settings, tail, (&target, None), &after);
    let mut fixture = Asked {
        flow,
        record,
        candidates: before,
        now: asked.clone(),
    };
    fixture.reply(&asked, Some(response(&[0.9, 0.9])));

    let order = fixture.gate(now, &after).await;

    assert_eq!(order, RelatedOrder::Rank("jev_stale"));
}
