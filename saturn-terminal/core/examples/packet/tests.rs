//! 개발용 패킷 생성 예제. 두 실험 설계의 입력 예로 패킷이 나오는지 본다.

use serde_json::{Value, json};
use tempfile::TempDir;

use clap::Parser;

use crate::args::PacketArgs;
use crate::{Rendered, run};

// cost: time O(L), heap O(L), stack O(1)
// vars: L = 입출력 글자 수
// basis: estimate
fn run_with_stdin(args: &[&str], stdin: &str) -> anyhow::Result<Rendered> {
    let args = PacketArgs::try_parse_from(std::iter::once("packet").chain(args.iter().copied()))?;
    run(&args, &mut stdin.as_bytes())
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 입력 항목 수
// basis: estimate
/// claude-summary-handoff 설계의 표준 입력 한 줄.
fn stream_lines(events: &[Value]) -> String {
    events
        .iter()
        .enumerate()
        .map(|(index, event)| json!({"record_no": index + 1, "event": event}).to_string() + "\n")
        .collect()
}

fn user_input(text: &str) -> Value {
    json!({"type": "saturn_user_input", "text": text})
}

fn tool_call(id: &str, path: &str) -> Value {
    json!({"type": "assistant", "message": {"content": [
        {"type": "tool_use", "id": id, "name": "Read", "input": {"file_path": path}}
    ]}})
}

fn tool_result(id: &str, text: &str) -> Value {
    json!({"type": "user", "message": {"content": [
        {"type": "tool_result", "tool_use_id": id, "content": text}
    ]}})
}

fn agent_text(text: &str) -> Value {
    json!({"type": "assistant", "message": {"content": [{"type": "text", "text": text}]}})
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 입력 항목 수
// basis: estimate
fn write(dir: &TempDir, name: &str, text: &str) -> String {
    let path = dir.path().join(name);
    std::fs::write(&path, text).unwrap();
    path.display().to_string()
}

fn stream_events() -> Vec<Value> {
    vec![
        user_input("start the auth work"),
        tool_call("t1", "src/old.rs"),
        tool_result("t1", "old file body"),
        agent_text("read the old file"),
        user_input("fix login in src/auth.rs"),
        tool_call("t2", "src/auth.rs"),
        tool_result("t2", "auth file body"),
        agent_text("fixed it"),
    ]
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 입력 항목 수
// basis: estimate
#[test]
fn packet_stream_input_prints_packet_text() {
    let input = stream_lines(&stream_events());

    let rendered = run_with_stdin(&["--mode", "saturn", "--budget-tokens", "800"], &input).unwrap();

    let text = rendered.output;
    assert!(text.contains("## Conversation"));
    assert!(text.contains("User: start the auth work"));
    assert!(text.contains("User: fix login in src/auth.rs\nAgent: fixed it"));
    assert!(text.contains("## Earlier records"));
    assert!(text.contains("auth file body"));
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 입력 항목 수
// basis: estimate
#[test]
fn packet_provider_mode_puts_summary_first_and_uses_records_after_it() {
    let dir = TempDir::new().unwrap();
    let summary = write(&dir, "summary.txt", "SUMMARY of the early work");
    let input = stream_lines(&stream_events());

    let rendered = run_with_stdin(
        &[
            "--mode",
            "provider",
            "--budget-tokens",
            "800",
            "--summary-file",
            &summary,
            "--summary-record",
            "3",
        ],
        &input,
    )
    .unwrap();

    let text = rendered.output;
    let summary_at = text
        .find("SUMMARY of the early work")
        .expect("summary should be in the packet");
    let records_at = text
        .find("auth file body")
        .expect("records should follow the summary");
    assert!(summary_at < records_at);
    assert!(!text.contains("old file body"));
    assert!(!rendered.is_summary_fallback);
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 입력 항목 수
// basis: estimate
#[test]
fn packet_provider_mode_oversized_summary_falls_back_to_records() {
    let dir = TempDir::new().unwrap();
    let summary = write(&dir, "summary.txt", &"S".repeat(4_000));
    let input = stream_lines(&stream_events());

    let rendered = run_with_stdin(
        &[
            "--mode",
            "provider",
            "--budget-tokens",
            "800",
            "--summary-file",
            &summary,
            "--summary-record",
            "3",
        ],
        &input,
    )
    .unwrap();

    assert!(rendered.is_summary_fallback);
    assert!(!rendered.output.contains("SSSS"));
    assert!(rendered.output.contains("auth file body"));
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 입력 항목 수
// basis: estimate
#[test]
fn packet_provider_mode_without_summary_returns_error() {
    let error = run_with_stdin(&["--mode", "provider", "--budget-tokens", "800"], "").unwrap_err();

    assert!(error.to_string().contains("--summary-file"));
}

// cost: time O(L), heap O(L), stack O(1)
// vars: L = packet output size
// basis: estimate
#[test]
fn packet_fixed_file_replaces_open_items_and_keeps_every_input() {
    let dir = TempDir::new().unwrap();
    let fixed = write(
        &dir,
        "fixed.json",
        r#"{"open_items":[{"seq":4,"text":"pending work"}]}"#,
    );
    let input = stream_lines(&stream_events());

    let rendered = run_with_stdin(
        &[
            "--mode",
            "saturn",
            "--budget-tokens",
            "800",
            "--fixed-file",
            &fixed,
        ],
        &input,
    )
    .unwrap();

    assert!(rendered.output.contains("pending work"));
}

fn tool_record(seq: u64, path: &str, result: &str) -> Value {
    json!({"seq": seq, "session": 1, "kind": "tool", "tool": "read", "args": {"path": path}, "result": result})
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 입력 항목 수
// basis: estimate
/// ranked-handoff-quality 설계의 시나리오 파일 한 줄. 후보는 도구 호출 6개다.
fn scenario_file(dir: &TempDir) -> String {
    let mut record =
        vec![json!({"seq": 1, "session": 1, "kind": "user", "text": "fix the cache module"})];
    for seq in 2..8 {
        let filler = format!("cache note {seq} {}", "x".repeat(180));
        record.push(tool_record(seq, &format!("src/mod{seq}.rs"), &filler));
    }
    record.push(json!({"seq": 8, "session": 2, "kind": "user", "text": "finish the cache module"}));
    let line = json!({"scenario_id": "s01", "record": record}).to_string();
    write(dir, "scenarios.jsonl", &format!("{line}\n"))
}

fn run_scenario(scenarios: &str, extra: &[&str]) -> Value {
    run_scenario_with_budget(scenarios, "350", extra)
}

fn run_scenario_with_budget(scenarios: &str, budget: &str, extra: &[&str]) -> Value {
    let mut args = vec![
        "--scenarios",
        scenarios,
        "--scenario-id",
        "s01",
        "--budget",
        budget,
        "--k",
        "60",
    ];
    args.extend_from_slice(extra);
    let rendered = run_with_stdin(&args, "").unwrap();
    serde_json::from_str(&rendered.output).unwrap()
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 입력 항목 수
// basis: estimate
fn seqs(value: &Value) -> Vec<u64> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|seq| seq.as_u64().unwrap())
        .collect()
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 입력 항목 수
// basis: estimate
#[test]
fn packet_scenarios_input_prints_json_with_packet_and_rrf_order() {
    let dir = TempDir::new().unwrap();

    let result = run_scenario(&scenario_file(&dir), &["--condition", "rrf-only"]);

    assert!(
        result["packet"]
            .as_str()
            .unwrap()
            .contains("finish the cache module")
    );
    assert_eq!(seqs(&result["rrf_order"]).len(), 6);
    assert!(!seqs(&result["included"]).is_empty());
    assert_eq!(result["router_calls"], 0);
    assert_eq!(result["router_failures"], 0);
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 입력 항목 수
// basis: estimate
#[test]
fn packet_without_judgments_fills_in_rrf_order() {
    let dir = TempDir::new().unwrap();

    let result = run_scenario(&scenario_file(&dir), &[]);

    let rrf = seqs(&result["rrf_order"]);
    let included = seqs(&result["included"]);
    assert!(included.contains(&rrf[0]));
    assert_eq!(result["routed"], 0);
}

#[test]
fn archive_boundary_is_kept_when_fixed_text_uses_the_small_budget() {
    let dir = TempDir::new().unwrap();
    let result = run_scenario_with_budget(&scenario_file(&dir), "250", &[]);
    assert!(seqs(&result["included"]).is_empty());
    assert!(
        result["packet"]
            .as_str()
            .unwrap()
            .ends_with(saturn_core::sessions::packet::ARCHIVE_END)
    );
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 입력 항목 수
// basis: estimate
#[test]
fn packet_judgments_put_low_probability_item_before_unanswered() {
    let dir = TempDir::new().unwrap();
    let scenarios = scenario_file(&dir);
    let baseline = run_scenario_with_budget(&scenarios, "350", &[]);
    let last = *seqs(&baseline["rrf_order"]).last().unwrap();
    let judgments = write(
        &dir,
        "judgments.json",
        &json!({"compact": [{"seq": last, "probability": 0.1}]}).to_string(),
    );

    let result = run_scenario_with_budget(&scenarios, "350", &["--judgments", &judgments]);

    let note = format!("cache note {last} ");
    assert!(!baseline["packet"].as_str().unwrap().contains(&note));
    assert!(result["packet"].as_str().unwrap().contains(&note));
    assert_eq!(result["routed"], 1);
}

#[test]
fn packet_rrf_only_condition_ignores_judgments() {
    let dir = TempDir::new().unwrap();
    let scenarios = scenario_file(&dir);
    let baseline = run_scenario_with_budget(&scenarios, "350", &[]);
    let last = *seqs(&baseline["rrf_order"]).last().unwrap();
    let judgments = write(
        &dir,
        "judgments.json",
        &json!({"compact": [{"seq": last, "probability": 0.1}]}).to_string(),
    );
    let note = format!("cache note {last} ");

    let ignored = run_scenario_with_budget(
        &scenarios,
        "350",
        &["--judgments", &judgments, "--condition", "rrf-only"],
    );
    let used = run_scenario_with_budget(
        &scenarios,
        "350",
        &["--judgments", &judgments, "--condition", "router-only"],
    );

    assert!(used["packet"].as_str().unwrap().contains(&note));
    assert_eq!(used["routed"], 1);
    assert!(!ignored["packet"].as_str().unwrap().contains(&note));
    assert_eq!(ignored["routed"], 0);
    assert_eq!(ignored["included"], baseline["included"]);
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 입력 항목 수
// basis: estimate
#[test]
fn packet_rrf_router_ignores_verdicts_outside_top_n() {
    let dir = TempDir::new().unwrap();
    let scenarios = scenario_file(&dir);
    let rrf = seqs(&run_scenario_with_budget(&scenarios, "350", &[])["rrf_order"]);
    let last = *rrf.last().unwrap();
    let judgments = write(
        &dir,
        "judgments.json",
        &json!({"compact": [{"seq": last, "probability": 0.1}]}).to_string(),
    );
    let outside = [
        "--judgments",
        judgments.as_str(),
        "--condition",
        "rrf-router",
    ];

    let mut args_all = outside.to_vec();
    args_all.extend(["--top-n", "6"]);
    let mut args_none = outside.to_vec();
    args_none.extend(["--top-n", "0"]);
    let used = run_scenario_with_budget(&scenarios, "350", &args_all);
    let skipped = run_scenario_with_budget(&scenarios, "350", &args_none);

    let note = format!("cache note {last} ");
    assert!(used["packet"].as_str().unwrap().contains(&note));
    assert!(!skipped["packet"].as_str().unwrap().contains(&note));
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 입력 항목 수
// basis: estimate
#[test]
fn packet_constraint_judgment_goes_to_fixed_zone() {
    let dir = TempDir::new().unwrap();
    let scenarios = write(
        &dir,
        "scenarios.jsonl",
        &(json!({"scenario_id": "s01", "record": [
            {"seq": 1, "session": 1, "kind": "user", "text": "never touch the billing module"},
            {"seq": 2, "session": 1, "kind": "user", "text": "fix the cache module"}
        ]})
        .to_string()
            + "\n"),
    );
    let judgments = write(
        &dir,
        "judgments.json",
        &json!({"constraints": [1]}).to_string(),
    );

    let result = run_scenario(&scenarios, &["--judgments", &judgments]);

    let packet = result["packet"].as_str().unwrap();
    assert!(packet.contains("## Constraints and decisions\n\nnever touch the billing module"));
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 입력 항목 수
// basis: estimate
#[test]
fn packet_missing_scenario_returns_error() {
    let dir = TempDir::new().unwrap();
    let scenarios = scenario_file(&dir);

    let error = run_with_stdin(
        &[
            "--scenarios",
            &scenarios,
            "--scenario-id",
            "nope",
            "--budget",
            "250",
        ],
        "",
    )
    .unwrap_err();

    assert!(error.to_string().contains("scenario not found: nope"));
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 입력 항목 수
// basis: estimate
#[test]
fn packet_scenario_session_and_time_appear_before_items() {
    let dir = TempDir::new().unwrap();
    let scenarios = write(
        &dir,
        "scenarios.jsonl",
        &(json!({"scenario_id": "s01", "record": [
            {"seq": 1, "session": 1, "ts": "2026-09-12T10:00:00Z", "kind": "user", "text": "fix the cache module"},
            {"seq": 2, "session": 1, "ts": "2026-09-12T10:05:30Z", "kind": "tool", "tool": "read", "args": {"path": "src/a.rs"}, "result": "cache body"},
            {"seq": 3, "session": 2, "ts": "2026-09-13T09:00:00Z", "kind": "user", "text": "finish the cache module"}
        ]})
        .to_string()
            + "\n"),
    );

    let result = run_scenario_with_budget(&scenarios, "350", &[]);

    let packet = result["packet"].as_str().unwrap();
    assert!(packet.contains("### Session 1\n\n#2 2026-09-12T10:05Z read"));
    assert!(packet.contains(
        "### Session 2\n\n#3 2026-09-13T09:00Z [Finished] User: finish the cache module"
    ));
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 입력 항목 수
// basis: estimate
#[test]
fn packet_stream_without_time_writes_seq_only() {
    let input = stream_lines(&stream_events());

    let rendered = run_with_stdin(&["--mode", "saturn", "--budget-tokens", "800"], &input).unwrap();

    let text = rendered.output;
    assert!(text.contains("#5 [Finished] User: fix login in src/auth.rs\nAgent: fixed it"));
    assert!(text.contains("### Session 1\n\n#2 Read "));
}
