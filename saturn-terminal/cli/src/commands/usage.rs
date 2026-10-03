//! `saturn usage`: 사용량 조회. `/usage`와 같다.
//! 설계: docs/design/tui.md

use std::io::Write;

use anyhow::Context;
use saturn_protocol::rpc::{Notification, Request, UsageRange, UsageRow};
use saturn_tui::client::EngineClient;
use saturn_tui::i18n::{self, Lang};
use saturn_tui::view::usage::{cost_text, range_name};

use crate::args::UsageArgs;
use crate::commands::call;

// cost: time O(r), heap O(r), stack O(1), io r
// vars: r = 행 수
// basis: estimate
/// provider가 보고하지 않은 값은 0이 아닌 빈 값으로 쓴다.
///
/// # Errors
/// 연결이 끊기면 오류.
pub(crate) async fn run(
    lang: Lang,
    client: &mut EngineClient,
    args: &UsageArgs,
) -> anyhow::Result<()> {
    let folder = std::env::current_dir()
        .context(lang.tr(i18n::CLI_CURRENT_DIR_UNREADABLE))?
        .display()
        .to_string();
    let table = fetch(lang, client, args, folder).await?;
    let mut out = std::io::stdout().lock();
    write_table(&mut out, lang, table.0, &table.1)?;
    Ok(())
}

// cost: time O(r), heap O(r), stack O(1), io 2
// vars: r = 행 수
// basis: estimate
async fn fetch(
    lang: Lang,
    client: &mut EngineClient,
    args: &UsageArgs,
    folder: String,
) -> anyhow::Result<(UsageRange, Vec<UsageRow>)> {
    let request = Request::Usage {
        scope: args.range(),
        folder: Some(folder),
    };
    let mut table = None;
    call(lang, client, request, |notification| {
        if let Notification::Usage { range, rows } = notification {
            table = Some((range, rows));
        }
    })
    .await?;
    table.ok_or_else(|| anyhow::anyhow!(lang.tr(i18n::CLI_NO_USAGE_TABLE)))
}

// cost: time O(r), heap O(1), stack O(1), io r
// vars: r = 행 수
// basis: estimate
/// 칸은 탭으로 나눈다. 파이프로 읽기 쉽게 하기 위해서다. 보고되지 않은 값은 `-`.
fn write_table(
    out: &mut impl Write,
    lang: Lang,
    range: UsageRange,
    rows: &[UsageRow],
) -> std::io::Result<()> {
    writeln!(
        out,
        "{} · {}",
        lang.tr(i18n::USAGE_TITLE),
        range_name(lang, range)
    )?;
    let headers = [
        i18n::USAGE_WHO,
        i18n::USAGE_INPUT,
        i18n::USAGE_CACHE_READ,
        i18n::USAGE_CACHE_WRITE,
        i18n::USAGE_OUTPUT,
        i18n::USAGE_REASONING,
        i18n::USAGE_ROUTER_CALLS,
        i18n::USAGE_COST,
    ];
    let header: Vec<&str> = headers.iter().map(|key| lang.tr(key)).collect();
    writeln!(out, "{}", header.join("\t"))?;
    for row in rows {
        let mut cells = vec![row.who.clone()];
        cells.extend(
            row.tokens
                .iter()
                .map(|value| value.map_or_else(|| "-".to_owned(), |value| value.to_string())),
        );
        cells.push(row.router_calls.to_string());
        cells.push(cost_text(row.estimated_cost_micros));
        writeln!(out, "{}", cells.join("\t"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::testing::{FakeEngine, Reply};

    use super::*;

    fn row(who: &str, tokens: [Option<u64>; 5], cost: Option<u64>) -> UsageRow {
        UsageRow {
            who: who.to_owned(),
            tokens,
            router_calls: 2,
            estimated_cost_micros: cost,
            compactions: None,
            labels: None,
            turns: None,
        }
    }

    #[test]
    fn write_table_marks_unreported_values_with_dash_not_zero() {
        let rows = [row(
            "codex · gpt",
            [Some(10), None, None, Some(5), None],
            None,
        )];
        let mut out = Vec::new();

        write_table(&mut out, Lang::En, UsageRange::Day, &rows).unwrap();

        let text = String::from_utf8(out).unwrap();
        let line = text.lines().last().unwrap();
        assert_eq!(line, "codex · gpt\t10\t-\t-\t5\t-\t2\t-");
        assert!(text.lines().next().unwrap().ends_with("Last 24 hours"));
    }

    #[tokio::test]
    async fn fetch_sends_range_and_reads_usage_notification() {
        let rows = vec![row("router · jev", [None; 5], Some(1_500_000))];
        let engine = FakeEngine::start(vec![Reply::with(vec![Notification::Usage {
            range: UsageRange::Week,
            rows: rows.clone(),
        }])]);
        let mut client = engine.client().await;

        let table = fetch(
            Lang::En,
            &mut client,
            &UsageArgs {
                day: false,
                week: true,
            },
            "/work".to_owned(),
        )
        .await
        .unwrap();

        assert_eq!(table, (UsageRange::Week, rows));
        assert_eq!(
            engine.finish().await,
            vec![Request::Usage {
                scope: UsageRange::Week,
                folder: Some("/work".to_owned()),
            }]
        );
    }

    #[tokio::test]
    async fn fetch_without_usage_notification_is_error() {
        let engine = FakeEngine::start(vec![Reply::ok()]);
        let mut client = engine.client().await;

        let result = fetch(
            Lang::En,
            &mut client,
            &UsageArgs {
                day: false,
                week: false,
            },
            "/work".to_owned(),
        )
        .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn fetch_when_router_key_is_required_explains_how_to_set_it() {
        let engine = FakeEngine::start(vec![Reply::error(-32001, "no key")]);
        let mut client = engine.client().await;

        let error = fetch(
            Lang::En,
            &mut client,
            &UsageArgs {
                day: true,
                week: false,
            },
            "/work".to_owned(),
        )
        .await
        .unwrap_err();

        let message = error.to_string();
        assert!(message.contains("SATURN_KEY"));
        assert!(message.contains("router.key.command"));
    }
}
