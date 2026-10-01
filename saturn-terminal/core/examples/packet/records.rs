//! 두 실험 설계의 입력을 같은 기록 목록으로 읽는다.
//! 설계: docs/experiments/ranked-handoff-quality/design.md, docs/experiments/claude-summary-handoff/design.md

use anyhow::{Context, bail};
use serde::Deserialize;
use serde_json::Value;

/// 기록 한 건. 번호는 입력이 정한 것을 그대로 쓴다.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Record {
    pub(crate) seq: u64,
    /// 입력에 없으면 1이다.
    pub(crate) session: u64,
    /// unix 밀리초. 입력에 `ts`가 없으면 `None`이다.
    pub(crate) at_ms: Option<i64>,
    pub(crate) body: Body,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Body {
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
    session: Option<u64>,
    ts: Option<String>,
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
pub(crate) fn from_scenarios(text: &str, scenario_id: &str) -> anyhow::Result<Vec<Record>> {
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

// cost: time O(r), heap O(r), stack O(1)
// vars: r = 기록 한 건의 글자 수
// basis: estimate
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
    let at_ms = row.ts.as_deref().map(parse_utc_ms).transpose()?;
    Ok(Record {
        seq: row.seq,
        session: row.session.unwrap_or(1),
        at_ms,
        body,
    })
}

// cost: time O(1), heap O(1), stack O(1)
// basis: estimate
/// `2026-09-12T10:00:00Z` 꼴을 unix 밀리초로 바꾼다. 소수 초는 받지 않는다.
///
/// # Errors
/// 모양이 다르면 오류.
pub(crate) fn parse_utc_ms(text: &str) -> anyhow::Result<i64> {
    let invalid = || anyhow::anyhow!("invalid UTC time: {text}");
    let (date, time) = text
        .strip_suffix('Z')
        .and_then(|t| t.split_once('T'))
        .ok_or_else(invalid)?;
    let number = |part: Option<&str>| part.and_then(|p| p.parse::<i64>().ok()).ok_or_else(invalid);
    let mut date = date.split('-');
    let (year, month, day) = (
        number(date.next())?,
        number(date.next())?,
        number(date.next())?,
    );
    let mut time = time.split(':');
    let (hour, minute, second) = (
        number(time.next())?,
        number(time.next())?,
        number(time.next())?,
    );
    let shifted_year = year - i64::from(month <= 2);
    let era = shifted_year.div_euclid(400);
    let year_of_era = shifted_year.rem_euclid(400);
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    Ok(((days * 24 + hour) * 60 + minute) * 60_000 + second * 1_000)
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
    fn from_scenarios_reads_session_and_time() {
        let text = r#"{"scenario_id":"s","record":[{"seq":1,"session":2,"ts":"2026-09-12T10:00:00Z","kind":"user","text":"x"},{"seq":2,"kind":"user","text":"y"}]}"#;

        let records = from_scenarios(text, "s").unwrap();

        assert_eq!(
            (records[0].session, records[0].at_ms),
            (2, Some(1_789_207_200_000))
        );
        assert_eq!((records[1].session, records[1].at_ms), (1, None));
    }

    #[test]
    fn parse_utc_ms_handles_leap_day_and_rejects_bad_shape() {
        assert_eq!(
            parse_utc_ms("2024-02-29T00:00:00Z").unwrap(),
            1_709_164_800_000
        );
        assert_eq!(parse_utc_ms("1970-01-01T00:00:01Z").unwrap(), 1_000);
        assert!(parse_utc_ms("2026-09-12 10:00").is_err());
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
