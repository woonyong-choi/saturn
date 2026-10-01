//! claude-summary-handoff 설계의 표준 입력: 줄마다 `{"record_no", "event"}`와 선택 필드 `session`, `ts`. `event`는 Claude Code stream-json 이벤트이거나 `saturn_user_input`.
//! 설계: docs/experiments/claude-summary-handoff/design.md

use std::collections::HashMap;

use anyhow::Context;
use serde::Deserialize;
use serde_json::Value;
use tracing::warn;

use crate::records::{Body, Record, parse_utc_ms};

#[derive(Debug, Deserialize)]
struct Line {
    record_no: u64,
    /// 없으면 1이다.
    session: Option<u64>,
    /// `2026-09-12T10:00:00Z` 꼴. 없으면 시각 없이 기록 번호만 적힌다.
    ts: Option<String>,
    event: Value,
}

// cost: time O(L), heap O(L), stack O(1)
// vars: L = 입력 글자 수
// basis: estimate
/// 사용자 입력, 에이전트 글, 도구 호출과 그 결과를 기록으로 옮긴다. 결과가 없는 호출은 끝나지 않은 항목이다.
/// 같은 이벤트의 둘째 이후 도구 호출은 기록 번호가 겹쳐 건너뛴다.
///
/// # Errors
/// 줄이 JSON이 아니거나 `record_no`, `event`가 없으면 오류.
pub(crate) fn from_stream(text: &str) -> anyhow::Result<Vec<Record>> {
    let mut records: Vec<Record> = Vec::new();
    let mut pending: HashMap<String, usize> = HashMap::new();
    for (index, line) in text
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
    {
        let line: Line = serde_json::from_str(line)
            .with_context(|| format!("failed to parse record line {}", index + 1))?;
        match line.event["type"].as_str() {
            Some("saturn_user_input") => {
                let text = line.event["text"].as_str().unwrap_or_default();
                records.push(record_of(&line, Body::User(text.to_owned()))?);
            }
            Some("assistant") => add_assistant(&mut records, &mut pending, &line)?,
            Some("user") => add_results(&mut records, &pending, &line.event),
            _ => {}
        }
    }
    Ok(records)
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 입력 항목 수
// basis: estimate
fn add_assistant(
    records: &mut Vec<Record>,
    pending: &mut HashMap<String, usize>,
    line: &Line,
) -> anyhow::Result<()> {
    let parts = line.event["message"]["content"]
        .as_array()
        .map_or(&[][..], Vec::as_slice);
    let answer: String = parts
        .iter()
        .filter(|part| part["type"] == "text")
        .filter_map(|part| part["text"].as_str())
        .collect();
    if !answer.is_empty() {
        records.push(record_of(line, Body::Agent(answer))?);
    }
    let mut calls = parts.iter().filter(|part| part["type"] == "tool_use");
    let Some(call) = calls.next() else {
        return Ok(());
    };
    if calls.next().is_some() {
        warn!(
            record_no = line.record_no,
            "event has several tool calls; only the first is kept"
        );
    }
    let id = call["id"].as_str().unwrap_or_default().to_owned();
    pending.insert(id, records.len());
    records.push(record_of(
        line,
        Body::Tool {
            name: call["name"].as_str().unwrap_or_default().to_owned(),
            args: call["input"].clone(),
            result: None,
        },
    )?);
    Ok(())
}

// cost: time O(1), heap O(1), stack O(1)
// basis: estimate
fn record_of(line: &Line, body: Body) -> anyhow::Result<Record> {
    Ok(Record {
        seq: line.record_no,
        session: line.session.unwrap_or(1),
        at_ms: line.ts.as_deref().map(parse_utc_ms).transpose()?,
        body,
    })
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 입력 항목 수
// basis: estimate
fn add_results(records: &mut [Record], pending: &HashMap<String, usize>, event: &Value) {
    let Some(parts) = event["message"]["content"].as_array() else {
        return;
    };
    for part in parts.iter().filter(|part| part["type"] == "tool_result") {
        let Some(&index) = part["tool_use_id"].as_str().and_then(|id| pending.get(id)) else {
            continue;
        };
        if let Body::Tool { result, .. } = &mut records[index].body {
            *result = Some(result_text(&part["content"]));
        }
    }
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 입력 항목 수
// basis: estimate
fn result_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| part["text"].as_str())
            .collect(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STREAM: &str = concat!(
        r#"{"record_no":1,"event":{"type":"saturn_user_input","text":"fix login"}}"#,
        "\n",
        r#"{"record_no":2,"event":{"type":"system","subtype":"init"}}"#,
        "\n",
        r#"{"record_no":3,"event":{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"src/a.rs"}}]}}}"#,
        "\n",
        r#"{"record_no":4,"event":{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":[{"type":"text","text":"fn a() {}"}]}]}}}"#,
        "\n",
        r#"{"record_no":5,"event":{"type":"assistant","message":{"content":[{"type":"text","text":"fixed"},{"type":"tool_use","id":"t2","name":"Bash","input":{"command":"cargo test"}}]}}}"#,
        "\n",
    );

    // cost: time O(n), heap O(n), stack O(1)
    // vars: n = 입력 항목 수
    // basis: estimate
    #[test]
    fn from_stream_pairs_results_and_keeps_unfinished_calls() {
        let records = from_stream(STREAM).unwrap();

        let seqs: Vec<u64> = records.iter().map(|record| record.seq).collect();
        assert_eq!(seqs, vec![1, 3, 5, 5]);
        assert_eq!(records[0].body, Body::User("fix login".into()));
        assert!(
            matches!(&records[1].body, Body::Tool { name, result: Some(r), .. } if name == "Read" && r == "fn a() {}")
        );
        assert_eq!(records[2].body, Body::Agent("fixed".into()));
        assert!(
            matches!(&records[3].body, Body::Tool { name, result: None, .. } if name == "Bash")
        );
    }

    #[test]
    fn from_stream_string_result_content_is_kept() {
        let text = concat!(
            r#"{"record_no":1,"event":{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t","name":"Bash","input":{}}]}}}"#,
            "\n",
            r#"{"record_no":2,"event":{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t","content":"ok"}]}}}"#,
            "\n",
        );

        let records = from_stream(text).unwrap();

        assert!(matches!(&records[0].body, Body::Tool { result: Some(r), .. } if r == "ok"));
    }

    #[test]
    fn from_stream_broken_line_returns_error_with_line_number() {
        let error = from_stream("{\"record_no\":1,\"event\":{}}\nnot json").unwrap_err();

        assert_eq!(error.to_string(), "failed to parse record line 2");
    }
}
