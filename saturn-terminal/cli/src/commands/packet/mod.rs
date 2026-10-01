//! 개발용 `packet` 명령. 실험 수집기가 `SATURN_PACKET_CMD`로 불러 `build_packet`과 후보 순위를 쓴다. engine에 붙지 않는다.
//! 설계: docs/experiments/ranked-handoff-quality/design.md, docs/experiments/claude-summary-handoff/design.md

mod assemble;
mod providers;
mod records;

use std::io::{Read, Write};
use std::path::Path;

use anyhow::{Context, bail};
use saturn_core::sessions::packet::{
    Entry, PacketOutcome, build_packet, build_packet_with_summary,
};
use saturn_protocol::ids::LedgerSeq;
use serde_json::json;

use self::assemble::{Judgments, OrderRule, assemble, budget_for};
use self::records::Record;
use crate::args::{PacketArgs, PacketCondition, PacketFormat, PacketMode};

// cost: time O(L), heap O(L), stack O(1), io 1
// vars: L = 입출력 글자 수
// basis: estimate
/// 패킷을 표준 출력에 쓴다. 요약이 예산을 넘어 원문으로 채웠으면 표준 오류에 `summary_fallback`을 쓴다.
///
/// # Errors
/// 입력을 읽을 수 없거나, `provider` 모드에 요약이 없거나, 고정 구역이 `P_hard`도 넘으면 오류.
pub(crate) fn run(args: &PacketArgs) -> anyhow::Result<()> {
    let records = read_records(args)?;
    let judgments = read_judgments(args.judgments.as_deref())?;
    let summary = read_summary(args)?;
    let rendered = render(args, &records, &judgments, summary)?;
    if rendered.is_summary_fallback {
        eprintln!("summary_fallback");
    }
    let mut out = std::io::stdout().lock();
    out.write_all(rendered.output.as_bytes())
        .context("failed to write packet")?;
    Ok(())
}

struct Rendered {
    output: String,
    is_summary_fallback: bool,
}

// cost: time O(L + c log c), heap O(L), stack O(1)
// vars: L = 기록 글자 수, c = 후보 수
// basis: estimate
fn render(
    args: &PacketArgs,
    records: &[Record],
    judgments: &Judgments,
    summary: Option<Entry>,
) -> anyhow::Result<Rendered> {
    let after = summary.as_ref().map(|entry| entry.seq.0);
    let rule = OrderRule {
        condition: args.condition.unwrap_or(if judgments.compact.is_empty() {
            PacketCondition::RrfOnly
        } else {
            PacketCondition::JudgeOnly
        }),
        k: args.k,
        top_n: args.top_n,
        keep_threshold: args.keep_threshold,
    };
    let assembled = assemble(records, after, judgments, &rule);
    let budget = budget_for(args.budget_tokens);
    let outcome = match &summary {
        Some(entry) => build_packet_with_summary(&assembled.source, &budget, entry),
        None => build_packet(&assembled.source, &budget),
    };
    let PacketOutcome::Ready(packet) = outcome else {
        bail!("fixed zone exceeds the hard limit; packet deferred");
    };
    let format = args.format.unwrap_or(if args.scenarios.is_some() {
        PacketFormat::Json
    } else {
        PacketFormat::Text
    });
    let output = match format {
        PacketFormat::Text => packet.text.clone(),
        PacketFormat::Json => json!({
            "packet": packet.text,
            "tokens": packet.tokens,
            "included": seqs(&packet.included),
            "rrf_order": seqs(&assembled.rrf_order),
            "judge_calls": 0,
            "judge_failures": 0,
            "judged": assembled.judged,
            "is_over_limit": packet.is_over_limit,
            "is_summary_used": packet.is_summary_used,
        })
        .to_string(),
    };
    Ok(Rendered {
        output,
        is_summary_fallback: summary.is_some() && !packet.is_summary_used,
    })
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 입력 항목 수
// basis: estimate
fn seqs(seqs: &[LedgerSeq]) -> Vec<u64> {
    seqs.iter().map(|seq| seq.0).collect()
}

// cost: time O(L), heap O(L), stack O(1), io 1
// vars: L = 입출력 글자 수
// basis: estimate
fn read_records(args: &PacketArgs) -> anyhow::Result<Vec<Record>> {
    if let (Some(path), Some(id)) = (&args.scenarios, &args.scenario_id) {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read scenarios: {}", path.display()))?;
        return records::from_scenarios(&text, id);
    }
    let mut text = String::new();
    std::io::stdin()
        .read_to_string(&mut text)
        .context("failed to read records from stdin")?;
    providers::claude::from_stream(&text)
}

// cost: time O(L), heap O(L), stack O(1), io 1
// vars: L = 입출력 글자 수
// basis: estimate
fn read_judgments(path: Option<&Path>) -> anyhow::Result<Judgments> {
    let Some(path) = path else {
        return Ok(Judgments::default());
    };
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read judgments: {}", path.display()))?;
    serde_json::from_str(&text)
        .with_context(|| format!("failed to parse judgments: {}", path.display()))
}

// cost: time O(L), heap O(L), stack O(1), io 1
// vars: L = 입출력 글자 수
// basis: estimate
fn read_summary(args: &PacketArgs) -> anyhow::Result<Option<Entry>> {
    let (Some(path), Some(record)) = (&args.summary_file, args.summary_record) else {
        if args.mode == PacketMode::Provider {
            bail!("provider mode needs --summary-file and --summary-record");
        }
        return Ok(None);
    };
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read summary: {}", path.display()))?;
    Ok(Some(Entry {
        seq: LedgerSeq(record),
        text,
    }))
}
