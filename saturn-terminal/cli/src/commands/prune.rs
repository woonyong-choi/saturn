//! `saturn prune`: 기록 정리. `--yes` 없이는 지울 채팅을 미리 보이기만 한다.
//! 설계: docs/design/records.md

use std::io::Write;

use saturn_protocol::envelope::ErrorKind;
use saturn_protocol::rpc::{ChatListItem, PruneSkipReason, PruneSkipped, QueryResult, Request};
use saturn_tui::client::{ClientError, EngineClient};
use saturn_tui::i18n::{self, Lang};

use crate::args::PruneArgs;
use crate::commands::call;
use crate::exit::{Exit, ExitCode};

/// engine가 보낸 정리 결과나 미리보기.
struct Report {
    chats: Vec<ChatListItem>,
    skipped: Vec<PruneSkipped>,
    rows: u64,
    /// 미리보기가 알린 확인 번호. 지운 결과에는 없다.
    plan: Option<String>,
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
    let called = call(
        lang,
        client,
        Request::Prune {
            yes: args.yes,
            plan: args.plan.clone(),
            all: args.yes && args.plan.is_none(),
        },
        drop,
    )
    .await;
    let result = match called {
        Ok(result) => result,
        Err(error) => return Err(prune_error(lang, error)),
    };
    let report = match result {
        Some(QueryResult::PrunePreview {
            chats,
            skipped,
            rows,
            plan,
        }) => Report {
            chats,
            skipped,
            rows,
            plan: Some(plan),
        },
        Some(QueryResult::Pruned {
            chats,
            skipped,
            rows,
        }) => Report {
            chats,
            skipped,
            rows,
            plan: None,
        },
        _ => {
            return Err(Exit::error(
                ExitCode::EngineInternal,
                lang.tr(i18n::CLI_NO_PRUNE_ANSWER),
            ));
        }
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
            kind: Some(ErrorKind::Config),
            ..
        })
    );
    let unknown_plan = matches!(
        error.downcast_ref::<ClientError>(),
        Some(ClientError::Rejected {
            kind: Some(ErrorKind::NotFound),
            ..
        })
    );
    if no_retention {
        Exit::error(ExitCode::Config, lang.tr(i18n::CLI_PRUNE_NO_RETENTION))
    } else if unknown_plan {
        Exit::error(ExitCode::NotFound, lang.tr(i18n::CLI_PRUNE_PLAN_UNKNOWN))
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
        let plan = report.plan.as_deref().unwrap_or_default();
        push_line(
            &mut out,
            &lang.tr(i18n::CLI_PRUNE_PREVIEW).replace("{plan}", plan),
        );
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
        PruneSkipReason::UsedSincePreview => i18n::CLI_SKIP_USED_SINCE_PREVIEW,
    }
}

#[cfg(test)]
mod tests {
    use saturn_protocol::envelope::INVALID_PARAMS;
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
            plan: Some("abc123".to_owned()),
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
        assert_eq!(
            lines[4],
            "Nothing was deleted · To delete, run: saturn prune --yes --plan abc123"
        );
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
            plan: None,
        };

        assert_eq!(
            render(Lang::En, &empty, false).lines().next(),
            Some("No chats to delete")
        );
    }

    #[tokio::test]
    async fn run_sends_the_prune_request_for_each_flag_combination() {
        struct Case {
            name: &'static str,
            args: PruneArgs,
            reply: Reply,
            expected: Request,
        }
        let pruned = || {
            Reply::result(QueryResult::Pruned {
                chats: vec![chat(4)],
                skipped: Vec::new(),
                rows: 3,
            })
        };
        let cases = vec![
            Case {
                name: "without yes it asks for the preview only and reads the preview",
                args: PruneArgs {
                    yes: false,
                    plan: None,
                },
                reply: Reply::result(QueryResult::PrunePreview {
                    chats: vec![chat(4)],
                    skipped: Vec::new(),
                    rows: 3,
                    plan: "abc123".to_owned(),
                }),
                expected: Request::Prune {
                    yes: false,
                    plan: None,
                    all: false,
                },
            },
            Case {
                name: "with yes it deletes and reads the result",
                args: PruneArgs {
                    yes: true,
                    plan: None,
                },
                reply: pruned(),
                expected: Request::Prune {
                    yes: true,
                    plan: None,
                    all: true,
                },
            },
            Case {
                name: "with a plan it sends the previewed id back",
                args: PruneArgs {
                    yes: true,
                    plan: Some("abc123".to_owned()),
                },
                reply: pruned(),
                expected: Request::Prune {
                    yes: true,
                    plan: Some("abc123".to_owned()),
                    all: false,
                },
            },
        ];

        for case in cases {
            let engine = FakeEngine::start(vec![case.reply]);
            let mut client = engine.client().await;

            run(Lang::En, &mut client, &case.args)
                .await
                .unwrap_or_else(|error| panic!("{}: {error:?}", case.name));

            assert_eq!(engine.finish().await, vec![case.expected], "{}", case.name);
        }
    }

    #[tokio::test]
    async fn run_with_an_unknown_plan_says_to_preview_again() {
        let engine = FakeEngine::start(vec![Reply::error_of_kind(
            INVALID_PARAMS,
            ErrorKind::NotFound,
            "unknown prune plan",
        )]);
        let mut client = engine.client().await;

        let error = run(
            Lang::En,
            &mut client,
            &PruneArgs {
                yes: true,
                plan: Some("old".to_owned()),
            },
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("run the preview again"));
        assert_eq!(crate::exit::of(&error), ExitCode::NotFound);
    }

    #[tokio::test]
    async fn run_without_a_prune_answer_is_error() {
        let engine = FakeEngine::start(vec![Reply::ok()]);
        let mut client = engine.client().await;

        let error = run(
            Lang::En,
            &mut client,
            &PruneArgs {
                yes: true,
                plan: None,
            },
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("without a prune result"));
    }

    #[tokio::test]
    async fn run_without_a_retention_setting_explains_how_to_set_it() {
        let engine = FakeEngine::start(vec![Reply::error_of_kind(
            INVALID_PARAMS,
            ErrorKind::Config,
            "no retention",
        )]);
        let mut client = engine.client().await;

        let error = run(
            Lang::En,
            &mut client,
            &PruneArgs {
                yes: true,
                plan: None,
            },
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("retention.max_age_days"));
        assert_eq!(crate::exit::of(&error), ExitCode::Config);
    }

    #[tokio::test]
    async fn run_when_engine_rejects_is_error() {
        let engine = FakeEngine::start(vec![Reply::error(-32601, "unsupported: Prune")]);
        let mut client = engine.client().await;

        let result = run(
            Lang::En,
            &mut client,
            &PruneArgs {
                yes: true,
                plan: None,
            },
        )
        .await;

        assert!(result.is_err());
    }
}
