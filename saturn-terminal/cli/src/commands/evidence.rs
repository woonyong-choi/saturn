//! `saturn evidence`: 에이전트 작업 안에서 이 채팅의 기록을 찾고, 패킷에서 생략된 기록의 원문을 번호로 다시 읽는다.
//! 떠 있는 engine에 출입증으로 접속하고, 출입증을 준 채팅의 기록만 본다. 결과는 파이프로 읽기 쉬운 글 줄이다.
//! 설계: docs/design/context-selection.md#근거-검색과-원문-조회

use std::io::Write;
use std::path::Path;

use anyhow::Context;
use saturn_protocol::ids::{ChatId, LedgerSeq};
use saturn_protocol::rpc::{
    EVIDENCE_UNREACHABLE_MARKER, EvidenceItem, EvidenceKind, QueryResult, Request,
};
use saturn_tui::client::EngineClient;
use saturn_tui::i18n::{self, Lang};

use crate::args::{EvidenceCommand, EvidenceReadArgs, EvidenceTarget};
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
    let request = match command {
        EvidenceCommand::Search(args) => Request::EvidenceSearch {
            pass: pass.to_owned(),
            query: args.query(),
            limit: args.limit,
        },
        EvidenceCommand::Read(args) => read_request(pass, args)?,
    };
    let mut client = EngineClient::connect(socket)
        .await
        .context(unreachable_message(lang, command))
        .map_err(|error| Exit::wrap(ExitCode::EngineUnavailable, error))?;
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
            kind,
            chat,
        }) => {
            let EvidenceCommand::Read(args) = command else {
                return Err(Exit::error(
                    ExitCode::EngineInternal,
                    lang.tr(i18n::CLI_EVIDENCE_NO_ANSWER),
                ));
            };
            if !matches_read_response(&args.id, (kind, chat, id, &hash)) {
                return Err(Exit::error(
                    ExitCode::EngineInternal,
                    "evidence read returned a different reference",
                ));
            }
            let typed = match &args.id {
                EvidenceTarget::Tool(_) => None,
                EvidenceTarget::Typed { kind, chat, .. } => Some((*kind, *chat)),
            };
            write_record(
                &mut out,
                (id, &hash, chars),
                (offset, next_offset),
                (&text, typed),
            )?;
        }
        _ => {
            return Err(Exit::error(
                ExitCode::EngineInternal,
                lang.tr(i18n::CLI_EVIDENCE_NO_ANSWER),
            ));
        }
    }
    Ok(())
}

fn matches_read_response(
    target: &EvidenceTarget,
    (kind, chat, id, hash): (EvidenceKind, Option<ChatId>, LedgerSeq, &str),
) -> bool {
    match target {
        EvidenceTarget::Tool(expected) => kind == EvidenceKind::Tool && id.0 == *expected,
        EvidenceTarget::Typed {
            kind: expected_kind,
            chat: expected_chat,
            id: expected_id,
            hash: expected_hash,
        } => {
            kind == *expected_kind
                && chat == Some(*expected_chat)
                && id.0 == *expected_id
                && hash == expected_hash
        }
    }
}

/// 옛 번호면 종류 없는 요청이고, 종류별 참조면 종류, 채팅, 번호, 해시를 모두 싣는다. 참조 안의 해시와 `--hash`가 다르면 오류다.
fn read_request(pass: &str, args: &EvidenceReadArgs) -> anyhow::Result<Request> {
    let (id, hash, kind, chat) = match &args.id {
        EvidenceTarget::Tool(id) => (*id, args.hash.clone(), None, None),
        EvidenceTarget::Typed {
            kind,
            chat,
            id,
            hash,
        } => {
            if args.hash.as_ref().is_some_and(|other| other != hash) {
                return Err(Exit::error(
                    ExitCode::Usage,
                    "--hash differs from the hash inside the reference",
                ));
            }
            (*id, Some(hash.clone()), Some(*kind), Some(*chat))
        }
    };
    Ok(Request::EvidenceRead {
        pass: pass.to_owned(),
        id: LedgerSeq(id),
        hash,
        offset: args.offset,
        limit: args.limit,
        kind,
        chat,
    })
}

/// 오류 첫머리가 `EVIDENCE_UNREACHABLE_MARKER (read 13)` 꼴이어야 engine이 닿지 못한 시도를 센다. 종류별 참조도 번호만 쓴다.
fn unreachable_message(lang: Lang, command: &EvidenceCommand) -> String {
    let what = match command {
        EvidenceCommand::Search(_) => "search".to_owned(),
        EvidenceCommand::Read(args) => format!("read {}", args.id.id()),
    };
    lang.tr(i18n::CLI_EVIDENCE_UNREACHABLE)
        .replace("{marker}", EVIDENCE_UNREACHABLE_MARKER)
        .replace("{what}", &what)
}

// cost: time O(n), heap O(1), stack O(1), io n
// vars: n = 후보 수
// basis: estimate
/// 첫 줄은 후보 집합 해시와 개수이고, 후보는 한 줄씩이다. 발췌의 줄바꿈은 `\n` 글자로 바꾼다.
/// 도구 호출은 옛 줄(`#번호 ... hash ...`) 그대로이고, 입력과 글은 첫 열이 `종류:채팅:번호:해시` 참조다.
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
        let excerpt = item.excerpt.replace('\n', "\\n");
        match item.kind {
            EvidenceKind::Tool => writeln!(
                out,
                "#{}\tat_ms {}\tchars {}\thash {}\texcerpt {}..{}\t{excerpt}",
                item.id.0, item.at_ms, item.chars, item.hash, item.excerpt_start, item.excerpt_end
            )?,
            kind => writeln!(
                out,
                "{}\tat_ms {}\tchars {}\texcerpt {}..{}\t{excerpt}",
                reference(kind, item.chat, item.id, &item.hash),
                item.at_ms,
                item.chars,
                item.excerpt_start,
                item.excerpt_end
            )?,
        }
    }
    Ok(())
}

/// `종류:채팅:번호:해시`. 읽을 때 그대로 넣는다.
fn reference(kind: EvidenceKind, chat: Option<ChatId>, id: LedgerSeq, hash: &str) -> String {
    format!(
        "{}:{}:{}:{hash}",
        kind.name(),
        chat.map_or(0, |chat| chat.0),
        id.0
    )
}

// cost: time O(n), heap O(1), stack O(1), io n
// vars: n = 원문 글자 수
// basis: estimate
/// 첫 줄이 번호, 해시, 전체 글자 수, 읽은 위치이고 빈 줄 뒤에 원문이다. 더 있으면 첫 줄에 다음 위치가 있다.
/// 종류별 참조로 읽었으면(`typed`) 번호와 해시 자리에 `종류:채팅:번호:해시` 참조가 온다.
fn write_record(
    out: &mut impl Write,
    (id, hash, chars): (LedgerSeq, &str, u64),
    (offset, next_offset): (u64, Option<u64>),
    (text, typed): (&str, Option<(EvidenceKind, ChatId)>),
) -> std::io::Result<()> {
    match typed {
        Some((kind, chat)) => write!(
            out,
            "{} chars {chars} offset {offset}",
            reference(kind, Some(chat), id, hash)
        )?,
        None => write!(out, "#{} hash {hash} chars {chars} offset {offset}", id.0)?,
    }
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
            kind: EvidenceKind::Tool,
            chat: Some(ChatId(3)),
            at_ms: 7,
            chars: 12,
            excerpt_start: 0,
            excerpt_end: 3,
            excerpt: "a\nb".to_owned(),
            hash: "ab".repeat(32),
        };

        write_candidates(&mut out, "set-hash", 3, &[item]).unwrap();

        assert_eq!(
            String::from_utf8(out).unwrap(),
            format!(
                "set set-hash candidates 3 shown 1\n#41\tat_ms 7\tchars 12\thash {}\texcerpt 0..3\ta\\nb\n",
                "ab".repeat(32)
            )
        );
    }

    #[test]
    fn a_record_header_names_the_next_offset_only_when_text_remains() {
        let mut more = Vec::new();
        let mut last = Vec::new();

        write_record(
            &mut more,
            (LedgerSeq(2), "h", 9),
            (0, Some(4)),
            ("body", None),
        )
        .unwrap();
        write_record(&mut last, (LedgerSeq(2), "h", 9), (4, None), ("rest", None)).unwrap();

        assert_eq!(
            String::from_utf8(more).unwrap(),
            "#2 hash h chars 9 offset 0 next_offset 4\n\nbody\n"
        );
        assert_eq!(
            String::from_utf8(last).unwrap(),
            "#2 hash h chars 9 offset 4\n\nrest\n"
        );
    }

    #[test]
    fn dialogue_candidates_and_typed_reads_carry_the_full_reference() {
        let hash = "cd".repeat(32);
        let item = |kind, id| EvidenceItem {
            id: LedgerSeq(id),
            kind,
            chat: Some(ChatId(3)),
            at_ms: 7,
            chars: 5,
            excerpt_start: 0,
            excerpt_end: 5,
            excerpt: "hi\nyo".to_owned(),
            hash: hash.clone(),
        };
        let mut list = Vec::new();
        let mut record = Vec::new();

        write_candidates(
            &mut list,
            "s",
            2,
            &[item(EvidenceKind::Input, 41), item(EvidenceKind::Text, 41)],
        )
        .unwrap();
        write_record(
            &mut record,
            (LedgerSeq(41), &hash, 5),
            (0, None),
            ("hi yo", Some((EvidenceKind::Text, ChatId(3)))),
        )
        .unwrap();

        assert_eq!(
            String::from_utf8(list).unwrap(),
            format!(
                "set s candidates 2 shown 2\n\
                 input:3:41:{hash}\tat_ms 7\tchars 5\texcerpt 0..5\thi\\nyo\n\
                 text:3:41:{hash}\tat_ms 7\tchars 5\texcerpt 0..5\thi\\nyo\n"
            )
        );
        assert_eq!(
            String::from_utf8(record).unwrap(),
            format!("text:3:41:{hash} chars 5 offset 0\n\nhi yo\n")
        );
    }

    fn read_args(id: EvidenceTarget, hash: Option<&str>) -> EvidenceReadArgs {
        EvidenceReadArgs {
            id,
            hash: hash.map(str::to_owned),
            offset: 0,
            limit: 100,
        }
    }

    #[test]
    fn a_legacy_number_sends_no_kind_and_a_reference_sends_kind_chat_and_hash() {
        let hash = "ef".repeat(32);
        let typed = EvidenceTarget::Typed {
            kind: EvidenceKind::Input,
            chat: ChatId(3),
            id: 41,
            hash: hash.clone(),
        };

        let legacy = read_request("p", &read_args(EvidenceTarget::Tool(41), Some("h"))).unwrap();
        let sent = read_request("p", &read_args(typed.clone(), None)).unwrap();
        let conflict = read_request("p", &read_args(typed, Some("other")));

        assert_eq!(
            legacy,
            Request::EvidenceRead {
                pass: "p".to_owned(),
                id: LedgerSeq(41),
                hash: Some("h".to_owned()),
                offset: 0,
                limit: 100,
                kind: None,
                chat: None,
            }
        );
        assert_eq!(
            sent,
            Request::EvidenceRead {
                pass: "p".to_owned(),
                id: LedgerSeq(41),
                hash: Some(hash),
                offset: 0,
                limit: 100,
                kind: Some(EvidenceKind::Input),
                chat: Some(ChatId(3)),
            }
        );
        assert!(conflict.is_err());
    }

    #[test]
    fn typed_read_rejects_an_old_engine_answer_for_the_same_number() {
        let hash = "ab".repeat(32);
        let target = EvidenceTarget::Typed {
            kind: EvidenceKind::Input,
            chat: ChatId(3),
            id: 41,
            hash: hash.clone(),
        };

        assert!(!matches_read_response(
            &target,
            (EvidenceKind::Tool, None, LedgerSeq(41), &hash)
        ));
        assert!(!matches_read_response(
            &target,
            (EvidenceKind::Input, Some(ChatId(4)), LedgerSeq(41), &hash)
        ));
        assert!(!matches_read_response(
            &target,
            (EvidenceKind::Input, Some(ChatId(3)), LedgerSeq(41), "stale")
        ));
        assert!(matches_read_response(
            &target,
            (EvidenceKind::Input, Some(ChatId(3)), LedgerSeq(41), &hash)
        ));
    }
}
