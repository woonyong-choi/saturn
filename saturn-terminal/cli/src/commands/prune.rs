//! `saturn prune`: 기록 정리. `--yes` 없이는 지울 채팅을 미리 보이기만 한다.
//! 설계: docs/design/records.md

use std::io::Write;

use saturn_protocol::envelope::INVALID_PARAMS;
use saturn_protocol::rpc::{ChatListItem, PruneSkipReason, PruneSkipped, QueryResult, Request};
use saturn_tui::client::{ClientError, EngineClient};
use saturn_tui::i18n::{self, Lang};

use crate::args::PruneArgs;
use crate::commands::call;

/// engine가 보낸 정리 결과나 미리보기.
struct Report {
    chats: Vec<ChatListItem>,
    skipped: Vec<PruneSkipped>,
    rows: u64,
}

// cost: time O(c), heap O(c), stack O(1), io 2
// vars: c = 지울 채팅 수와 남긴 채팅 수
// basis: estimate
/// 삭제 제외 대상(열린 입력, 활성 session 등)은 engine이 거른다.
///
/// # Errors
/// 정리 기준 설정이 없거나, engine이 거절했거나, 연결이 끊기면 오류.
pub(crate) async fn run(
    lang: Lang,
    client: &mut EngineClient,
    args: &PruneArgs,
) -> anyhow::Result<()> {
    let called = call(lang, client, Request::Prune { yes: args.yes }, drop).await;
    let result = match called {
        Ok(result) => result,
        Err(error) => return Err(prune_error(lang, error)),
    };
    let report = match result {
        Some(
            QueryResult::PrunePreview {
                chats,
                skipped,
                rows,
            }
            | QueryResult::Pruned {
                chats,
                skipped,
                rows,
            },
        ) => Report {
            chats,
            skipped,
            rows,
        },
        _ => anyhow::bail!(lang.tr(i18n::CLI_NO_PRUNE_ANSWER)),
    };
    write!(
        std::io::stdout().lock(),
        "{}",
        render(lang, &report, args.yes)
    )?;
    Ok(())
}

/// 기준 설정이 없다는 거절은 설정 방법을 안내한다.
fn prune_error(lang: Lang, error: anyhow::Error) -> anyhow::Error {
    let no_retention = matches!(
        error.downcast_ref::<ClientError>(),
        Some(ClientError::Rejected {
            code: INVALID_PARAMS,
            ..
        })
    );
    if no_retention {
        anyhow::anyhow!(lang.tr(i18n::CLI_PRUNE_NO_RETENTION))
    } else {
        error
    }
}

fn render(lang: Lang, report: &Report, is_deleted: bool) -> String {
    let mut out = String::new();
    if report.chats.is_empty() {
        push_line(&mut out, lang.tr(i18n::CLI_PRUNE_NOTHING));
    } else {
        let header = if is_deleted {
            i18n::CLI_PRUNE_DONE
        } else {
            i18n::CLI_PRUNE_PLAN
        };
        push_line(
            &mut out,
            &lang
                .tr(header)
                .replace("{chats}", &report.chats.len().to_string())
                .replace("{rows}", &report.rows.to_string()),
        );
        for chat in &report.chats {
            push_line(
                &mut out,
                &format!("  {}", saturn_tui::chat_summary(lang, chat, true)),
            );
        }
    }
    if !report.skipped.is_empty() {
        push_line(
            &mut out,
            &lang
                .tr(i18n::CLI_PRUNE_KEPT)
                .replace("{chats}", &report.skipped.len().to_string()),
        );
        for skipped in &report.skipped {
            let reasons: Vec<&str> = skipped
                .reasons
                .iter()
                .map(|reason| lang.tr(reason_phrase(*reason)))
                .collect();
            push_line(
                &mut out,
                &format!("  #{} · {}", skipped.chat.0, reasons.join(", ")),
            );
        }
    }
    if !is_deleted {
        push_line(&mut out, lang.tr(i18n::CLI_PRUNE_PREVIEW));
    }
    out
}

fn push_line(out: &mut String, line: &str) {
    out.push_str(line);
    out.push('\n');
}

fn reason_phrase(reason: PruneSkipReason) -> &'static str {
    match reason {
        PruneSkipReason::OpenInput => i18n::CLI_SKIP_OPEN_INPUT,
        PruneSkipReason::OpenRun => i18n::CLI_SKIP_OPEN_RUN,
        PruneSkipReason::PendingStop => i18n::CLI_SKIP_PENDING_STOP,
        PruneSkipReason::ActiveSession => i18n::CLI_SKIP_ACTIVE_SESSION,
        PruneSkipReason::WaitingSession => i18n::CLI_SKIP_WAITING_SESSION,
        PruneSkipReason::Attached => i18n::CLI_SKIP_ATTACHED,
    }
}

#[cfg(test)]
mod tests {
    use saturn_protocol::ids::ChatId;

    use crate::testing::{FakeEngine, Reply};

    use super::*;

    fn chat(id: u64) -> ChatListItem {
        ChatListItem {
            chat: ChatId(id),
            folder: "/work/a".to_owned(),
            name: Some("login fix".to_owned()),
            last_active_ms: 0,
            preview: Some("fix login".to_owned()),
            rows: None,
        }
    }

    fn report() -> Report {
        Report {
            chats: vec![chat(4)],
            skipped: vec![PruneSkipped {
                chat: ChatId(9),
                reasons: vec![PruneSkipReason::OpenInput, PruneSkipReason::Attached],
            }],
            rows: 12,
        }
    }

    #[test]
    fn preview_lists_what_would_be_deleted_and_what_stays_and_says_nothing_was_deleted() {
        let text = render(Lang::En, &report(), false);

        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "Chats to delete: 1 · Rows: 12");
        assert!(lines[1].starts_with("  #4 · "));
        assert!(lines[1].ends_with("/work/a · login fix · fix login"));
        assert_eq!(lines[2], "Chats kept: 1");
        assert_eq!(lines[3], "  #9 · open input, attached to a TUI");
        assert_eq!(lines[4], "Nothing was deleted · Run with --yes to delete");
    }

    #[test]
    fn deleted_report_says_what_was_deleted_without_the_preview_hint() {
        let text = render(Lang::En, &report(), true);

        assert!(text.starts_with("Deleted chats: 1 · Rows: 12\n"));
        assert!(!text.contains("Nothing was deleted"));
    }

    #[test]
    fn empty_report_says_there_is_nothing_to_delete() {
        let empty = Report {
            chats: Vec::new(),
            skipped: Vec::new(),
            rows: 0,
        };

        assert_eq!(
            render(Lang::En, &empty, false).lines().next(),
            Some("No chats to delete")
        );
    }

    #[tokio::test]
    async fn run_without_yes_asks_for_preview_only_and_reads_the_preview() {
        let engine = FakeEngine::start(vec![Reply::result(QueryResult::PrunePreview {
            chats: vec![chat(4)],
            skipped: Vec::new(),
            rows: 3,
        })]);
        let mut client = engine.client().await;

        run(Lang::En, &mut client, &PruneArgs { yes: false })
            .await
            .unwrap();

        assert_eq!(engine.finish().await, vec![Request::Prune { yes: false }]);
    }

    #[tokio::test]
    async fn run_with_yes_deletes_and_reads_the_result() {
        let engine = FakeEngine::start(vec![Reply::result(QueryResult::Pruned {
            chats: vec![chat(4)],
            skipped: Vec::new(),
            rows: 3,
        })]);
        let mut client = engine.client().await;

        run(Lang::En, &mut client, &PruneArgs { yes: true })
            .await
            .unwrap();

        assert_eq!(engine.finish().await, vec![Request::Prune { yes: true }]);
    }

    #[tokio::test]
    async fn run_without_a_prune_answer_is_error() {
        let engine = FakeEngine::start(vec![Reply::ok()]);
        let mut client = engine.client().await;

        let error = run(Lang::En, &mut client, &PruneArgs { yes: true })
            .await
            .unwrap_err();

        assert!(error.to_string().contains("without a prune result"));
    }

    #[tokio::test]
    async fn run_without_a_retention_setting_explains_how_to_set_it() {
        let engine = FakeEngine::start(vec![Reply::error(INVALID_PARAMS, "no retention")]);
        let mut client = engine.client().await;

        let error = run(Lang::En, &mut client, &PruneArgs { yes: true })
            .await
            .unwrap_err();

        assert!(error.to_string().contains("retention.max_age_days"));
    }

    #[tokio::test]
    async fn run_when_engine_rejects_is_error() {
        let engine = FakeEngine::start(vec![Reply::error(-32601, "unsupported: Prune")]);
        let mut client = engine.client().await;

        let result = run(Lang::En, &mut client, &PruneArgs { yes: true }).await;

        assert!(result.is_err());
    }
}
