//! `saturn` 실행 파일. engine 라이브러리에 링크하지 않고 `saturn-engine`을 띄워 소켓으로 붙는다.
//! 설계: docs/architecture.md

use std::io::Write;

use clap::FromArgMatches;
use saturn_tui::i18n::Lang;

use crate::args::{Cli, Command, OpenMode};
use crate::exit::{Exit, ExitCode};

mod args;
mod commands;
mod exit;
mod launch;
#[cfg(test)]
mod testing;

// cost: time O(1), heap O(1), stack O(1), io 3
// basis: estimate
/// 종료 코드는 `exit::ExitCode`가 정한다. 설계: docs/design/engine-lifecycle.md
#[tokio::main]
async fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let lang = Lang::detect();
    let (cli, mode) = match parse(lang) {
        Ok(parsed) => parsed,
        Err(error) => {
            let _ = error.print(); // 도움말과 오류 문구를 못 써도 종료 코드는 남는다
            let code = if error.use_stderr() {
                ExitCode::Usage
            } else {
                ExitCode::Success
            };
            return code.into();
        }
    };
    match run(lang, cli, mode).await {
        Ok(()) => ExitCode::Success.into(),
        Err(error) => {
            let _ = writeln!(std::io::stderr(), "Error: {error:?}"); // 쓰지 못해도 종료 코드는 남는다
            exit::of(&error).into()
        }
    }
}

fn parse(lang: Lang) -> Result<(Cli, OpenMode), clap::Error> {
    let cli = Cli::from_arg_matches(&args::command(lang).try_get_matches()?)?;
    let mode = cli.open_mode(lang)?;
    Ok((cli, mode))
}

async fn run(lang: Lang, cli: Cli, mode: OpenMode) -> anyhow::Result<()> {
    match (launch::origin(lang)?, &cli.command) {
        (launch::Origin::Child { pass, socket }, Some(Command::Evidence { command })) => {
            return commands::evidence::run(lang, command, &pass, &socket).await;
        }
        (launch::Origin::Child { pass, socket }, _) => {
            return commands::child::run(lang, &cli, pass, &socket).await;
        }
        (launch::Origin::Outside, Some(Command::Evidence { .. })) => {
            return Err(Exit::error(
                ExitCode::Usage,
                lang.tr(saturn_tui::i18n::CLI_EVIDENCE_NEEDS_PASS)
                    .replace("{pass}", saturn_protocol::rpc::PASS_ENV),
            ));
        }
        (launch::Origin::Outside, _) => {}
    }
    if cli.mode.is_some() {
        return Err(Exit::error(
            ExitCode::Usage,
            lang.tr(saturn_tui::i18n::CLI_MODE_NEEDS_PASS),
        ));
    }
    commands::chat::ensure_can_open(lang, mode)?;
    let add_dirs = commands::chat::resolve_add_dirs(lang, &cli.add_dir)?;
    let mut client = launch::connect_or_start(lang).await?;
    let chat = commands::chat::resolve_chat(lang, &mut client, mode).await?;

    match cli.command {
        None => {
            let plain = commands::chat::plain_override(cli.plain, std::env::var("NO_COLOR").ok());
            commands::chat::run(lang, &mut client, chat, &cli.config, add_dirs, plain).await
        }
        Some(Command::Prune(args)) => commands::prune::run(lang, &mut client, &args).await,
        Some(Command::Export(args)) => commands::export::run(lang, &mut client, &args).await,
        Some(Command::Usage(args)) => commands::usage::run(lang, &mut client, &args).await,
        Some(Command::Evidence { .. }) => {
            unreachable!("evidence is answered before the engine is opened")
        }
    }
}
