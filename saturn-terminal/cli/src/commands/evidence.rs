//! `saturn evidence`: 에이전트 작업 안에서 이 채팅의 기록을 찾고, 패킷에서 생략된 기록의 원문을 번호로 다시 읽는다.
//! 떠 있는 engine에 출입증으로 접속하고, 출입증을 준 채팅의 기록만 본다. 결과는 파이프로 읽기 쉬운 글 줄이다.
//! 설계: docs/design/context-selection.md#근거-검색과-원문-조회

use std::io::Write;
use std::path::Path;

use anyhow::Context;
use saturn_protocol::ids::LedgerSeq;
use saturn_protocol::rpc::{EvidenceItem, QueryResult, Request};
use saturn_tui::client::EngineClient;
use saturn_tui::i18n::{self, Lang};

use crate::args::EvidenceCommand;
use crate::commands::call;
use crate::exit::{Exit, ExitCode};

// cost: time O(n), heap O(n), stack O(1), io 2
// vars: n = 돌려받는 글자 수
// basis: estimate
/// 새 engine을 띄우지 않는다.
///
/// # Errors
/// engine이 떠 있지 않거나, engine이 거절하거나(출입증, 없는 번호, 읽기 범위, 바뀐 해시), 결과를 쓰지 못하면 오류.
pub(crate) async fn run(
    lang: Lang,
    command: &EvidenceCommand,
    pass: &str,
    socket: &Path,
) -> anyhow::Result<()> {
    let mut client = EngineClient::connect(socket)
        .await
        .context(lang.tr(i18n::CLI_CHILD_NO_ENGINE))
        .map_err(|error| Exit::wrap(ExitCode::EngineUnavailable, error))?;
    let request = match command {
        EvidenceCommand::Search(args) => Request::EvidenceSearch {
            pass: pass.to_owned(),
            query: args.query.clone(),
            limit: args.limit,
        },
        EvidenceCommand::Read(args) => Request::EvidenceRead {
            pass: pass.to_owned(),
            id: LedgerSeq(args.id),
            hash: args.hash.clone(),
            offset: args.offset,
            limit: args.limit,
        },
    };
    let result = call(lang, &mut client, request, drop).await?;
    let mut out = std::io::stdout().lock();
    match result {
        Some(QueryResult::EvidenceCandidates {
            set_hash,
            total,
            items,
        }) => write_candidates(&mut out, &set_hash, total, &items)?,
        Some(QueryResult::EvidenceRecord {
            id,
            hash,
            chars,
            offset,
            next_offset,
            text,
        }) => write_record(&mut out, (id, &hash, chars), (offset, next_offset), &text)?,
        _ => {
            return Err(Exit::error(
                ExitCode::EngineInternal,
                lang.tr(i18n::CLI_EVIDENCE_NO_ANSWER),
            ));
        }
    }
    Ok(())
}

// cost: time O(n), heap O(1), stack O(1), io n
// vars: n = 후보 수
// basis: estimate
/// 첫 줄은 후보 집합 해시와 개수이고, 후보는 한 줄씩이다. 발췌의 줄바꿈은 `\n` 글자로 바꾼다.
fn write_candidates(
    out: &mut impl Write,
    set_hash: &str,
    total: u32,
    items: &[EvidenceItem],
) -> std::io::Result<()> {
    writeln!(
        out,
        "set {set_hash} candidates {total} shown {}",
        items.len()
    )?;
    for item in items {
        writeln!(
            out,
            "#{}\tat_ms {}\tchars {}\thash {}\t{}",
            item.id.0,
            item.at_ms,
            item.chars,
            item.hash,
            item.excerpt.replace('\n', "\\n")
        )?;
    }
    Ok(())
}

// cost: time O(n), heap O(1), stack O(1), io n
// vars: n = 원문 글자 수
// basis: estimate
/// 첫 줄이 번호, 해시, 전체 글자 수, 읽은 위치이고 빈 줄 뒤에 원문이다. 더 있으면 첫 줄에 다음 위치가 있다.
fn write_record(
    out: &mut impl Write,
    (id, hash, chars): (LedgerSeq, &str, u64),
    (offset, next_offset): (u64, Option<u64>),
    text: &str,
) -> std::io::Result<()> {
    write!(out, "#{} hash {hash} chars {chars} offset {offset}", id.0)?;
    if let Some(next) = next_offset {
        write!(out, " next_offset {next}")?;
    }
    writeln!(out, "\n")?;
    out.write_all(text.as_bytes())?;
    writeln!(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_are_one_line_each_and_keep_the_hash_for_a_later_read() {
        let mut out = Vec::new();
        let item = EvidenceItem {
            id: LedgerSeq(41),
            at_ms: 7,
            chars: 12,
            excerpt: "a\nb".to_owned(),
            hash: "ab".repeat(32),
        };

        write_candidates(&mut out, "set-hash", 3, &[item]).unwrap();

        assert_eq!(
            String::from_utf8(out).unwrap(),
            format!(
                "set set-hash candidates 3 shown 1\n#41\tat_ms 7\tchars 12\thash {}\ta\\nb\n",
                "ab".repeat(32)
            )
        );
    }

    #[test]
    fn a_record_header_names_the_next_offset_only_when_text_remains() {
        let mut more = Vec::new();
        let mut last = Vec::new();

        write_record(&mut more, (LedgerSeq(2), "h", 9), (0, Some(4)), "body").unwrap();
        write_record(&mut last, (LedgerSeq(2), "h", 9), (4, None), "rest").unwrap();

        assert_eq!(
            String::from_utf8(more).unwrap(),
            "#2 hash h chars 9 offset 0 next_offset 4\n\nbody\n"
        );
        assert_eq!(
            String::from_utf8(last).unwrap(),
            "#2 hash h chars 9 offset 4\n\nrest\n"
        );
    }
}
