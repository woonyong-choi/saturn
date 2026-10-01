//! 두 실험 설계의 입력을 같은 기록 목록으로 읽는다.
//! 설계: docs/experiments/ranked-handoff-quality/design.md, docs/experiments/claude-summary-handoff/design.md

use anyhow::{Context, bail};
use serde::Deserialize;
use serde_json::Value;

/// 기록 한 건. 번호는 입력이 정한 것을 그대로 쓴다.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Record {
    pub(super) seq: u64,
    pub(super) body: Body,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Body {
    User(String),
    Agent(String),
    /// `result`가 `None`이면 끝나지 않은 호출이다.
    Tool {
        name: String,
        args: Value,
        result: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
struct ScenarioLine {
    scenario_id: String,
    record: Vec<ScenarioRecord>,
}

#[derive(Debug, Deserialize)]
struct ScenarioRecord {
    seq: u64,
    kind: String,
    text: Option<String>,
    tool: Option<String>,
    args: Option<Value>,
    result: Option<String>,
}

// cost: time O(L), heap O(L), stack O(1)
// vars: L = 시나리오 파일 글자 수
// basis: estimate
/// ranked-handoff-quality 설계의 `--scenarios` 파일(한 줄에 시나리오 하나)에서 `scenario_id`의 기록을 읽는다.
///
/// # Errors
/// 줄을 읽을 수 없거나, 시나리오가 없거나, 모르는 `kind`면 오류.
pub(super) fn from_scenarios(text: &str, scenario_id: &str) -> anyhow::Result<Vec<Record>> {
    for (index, line) in text
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
    {
        let scenario: ScenarioLine = serde_json::from_str(line)
            .with_context(|| format!("failed to parse scenario line {}", index + 1))?;
        if scenario.scenario_id == scenario_id {
            return scenario.record.into_iter().map(scenario_record).collect();
        }
    }
    bail!("scenario not found: {scenario_id}")
}

fn scenario_record(row: ScenarioRecord) -> anyhow::Result<Record> {
    let body = match row.kind.as_str() {
        "user" => Body::User(row.text.unwrap_or_default()),
        "agent" => Body::Agent(row.text.unwrap_or_default()),
        "tool" => Body::Tool {
            name: row.tool.unwrap_or_default(),
            args: row.args.unwrap_or(Value::Null),
            result: row.result,
        },
        other => bail!("unknown record kind at seq {}: {other}", row.seq),
    };
    Ok(Record { seq: row.seq, body })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCENARIOS: &str = concat!(
        r#"{"scenario_id":"s01","record":[{"seq":1,"kind":"user","text":"fix login"},"#,
        r#"{"seq":2,"kind":"tool","tool":"read","args":{"path":"src/a.rs"},"result":"body"},"#,
        r#"{"seq":3,"kind":"agent","text":"done"}]}"#,
        "\n",
        r#"{"scenario_id":"s02","record":[{"seq":1,"kind":"user","text":"other"}]}"#,
        "\n"
    );

    #[test]
    fn from_scenarios_picks_requested_scenario() {
        let records = from_scenarios(SCENARIOS, "s01").unwrap();

        assert_eq!(records.len(), 3);
        assert_eq!(records[0].body, Body::User("fix login".into()));
        assert_eq!(records[2].body, Body::Agent("done".into()));
        assert!(
            matches!(&records[1].body, Body::Tool { name, result: Some(r), .. } if name == "read" && r == "body")
        );
    }

    #[test]
    fn from_scenarios_unknown_id_returns_error() {
        let error = from_scenarios(SCENARIOS, "s99").unwrap_err();

        assert_eq!(error.to_string(), "scenario not found: s99");
    }

    #[test]
    fn from_scenarios_unknown_kind_returns_error() {
        let text = r#"{"scenario_id":"s","record":[{"seq":4,"kind":"weird"}]}"#;

        let error = from_scenarios(text, "s").unwrap_err();

        assert_eq!(error.to_string(), "unknown record kind at seq 4: weird");
    }

    #[test]
    fn from_scenarios_broken_line_returns_error_with_line_number() {
        let error = from_scenarios("{broken", "s").unwrap_err();

        assert_eq!(error.to_string(), "failed to parse scenario line 1");
    }
}
