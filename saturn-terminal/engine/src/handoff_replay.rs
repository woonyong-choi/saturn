//! 비공개 실제 기록을 현재 engine 패킷 경로로 재생하는 실험 진입점.

use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;

use super::*;

#[derive(Deserialize)]
struct ReplayRow {
    seq: u64,
    run: u64,
    session: u64,
    input: Option<String>,
    end: Option<String>,
    at_ms: i64,
    event: ProviderEvent,
}

#[derive(Deserialize)]
struct Case {
    id: String,
    rows: Vec<ReplayRow>,
}

#[derive(Deserialize)]
struct RecallRequest {
    id: String,
    case: String,
    query: String,
    max_chars: usize,
}

#[test]
#[ignore = "requires explicit private replay corpus, queries and output directory"]
fn export_recalled_evidence() {
    let input = PathBuf::from(std::env::var_os("SATURN_REPLAY_INPUT").unwrap());
    let output = PathBuf::from(std::env::var_os("SATURN_REPLAY_OUTPUT").unwrap());
    let queries = PathBuf::from(std::env::var_os("SATURN_RECALL_QUERIES").unwrap());
    let cases: Vec<Case> = serde_json::from_slice(&std::fs::read(input).unwrap()).unwrap();
    let queries: Vec<RecallRequest> =
        serde_json::from_slice(&std::fs::read(queries).unwrap()).unwrap();
    std::fs::create_dir_all(&output).unwrap();
    for request in queries {
        let case = cases.iter().find(|case| case.id == request.case).unwrap();
        let rows: Vec<_> = case.rows.iter().map(ledger_row).collect();
        let text = super::recall::recall_evidence(&rows, &[], &request.query, request.max_chars);
        let input = super::recall::attach_evidence(&request.query, text.as_deref());
        let path = output.join(format!("{}.json", request.id));
        assert!(!path.exists(), "replay output must not be overwritten");
        std::fs::write(
            path,
            serde_json::to_vec_pretty(&serde_json::json!({"text": text, "input": input})).unwrap(),
        )
        .unwrap();
    }
}

#[test]
#[ignore = "requires explicit private replay corpus and output directory"]
fn export_real_record_packets() {
    let input = PathBuf::from(std::env::var_os("SATURN_REPLAY_INPUT").unwrap());
    let output = PathBuf::from(std::env::var_os("SATURN_REPLAY_OUTPUT").unwrap());
    let cases: Vec<Case> = serde_json::from_slice(&std::fs::read(input).unwrap()).unwrap();
    std::fs::create_dir_all(&output).unwrap();
    for case in cases {
        export_case(&case, &output);
    }
}

fn export_case(case: &Case, output: &std::path::Path) {
    let rows: Vec<LedgerRow> = case.rows.iter().map(ledger_row).collect();
    for limit in [1_000, 4_000, 16_000] {
        let budget = ContextBudget {
            t_abs: limit * 10,
            safety_percent: 100,
            window: limit * 10,
            cache_read: 0.1,
            cache_write: 1.25,
            cache_ttl: Duration::from_secs(300),
            item_cap_percent: 30,
            constraint_slot_percent: 25,
            evidence_lookup: false,
        };
        let result = build_handoff(&rows, &[], &[], &Pending::default(), (&[], &[]), &budget);
        let value = match result {
            HandoffOutcome::Ready(packet) => serde_json::json!({
                "status": "ready", "text": packet.text, "estimated_tokens": packet.tokens,
                "over_limit": packet.is_over_limit,
            }),
            HandoffOutcome::Deferred { .. } => serde_json::json!({"status": "deferred"}),
            HandoffOutcome::Empty => serde_json::json!({"status": "empty"}),
        };
        let path = output.join(format!("{}-{limit}.json", case.id));
        assert!(!path.exists(), "replay output must not be overwritten");
        std::fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    }
}

fn ledger_row(row: &ReplayRow) -> LedgerRow {
    LedgerRow {
        seq: LedgerSeq(row.seq),
        run: RunId(row.run),
        session: SessionId(row.session),
        input: row.input.clone(),
        end: row.end.as_deref().map(|end| match end {
            "Completed" => RunEnd::Completed,
            "Failed" => RunEnd::Failed,
            "Stopped" => RunEnd::Stopped,
            _ => panic!("unexpected run end in replay corpus"),
        }),
        at_ms: row.at_ms,
        event: row.event.clone(),
    }
}
