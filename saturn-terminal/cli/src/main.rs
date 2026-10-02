//! `saturn` 실행 파일. engine 라이브러리에 링크하지 않고 `saturn-engine`을 띄워 소켓으로 붙는다.
//! 설계: docs/architecture.md

use clap::FromArgMatches;
use saturn_tui::i18n::Lang;

use crate::args::{Cli, Command, RouterCommand};

mod args;
mod commands;
mod launch;
#[cfg(test)]
mod testing;

// cost: time O(1), heap O(1), stack O(1), io 3
// basis: estimate
// TODO(#47): 종료 코드. 지금은 성공 0, 실패 1(`anyhow` 기본), 명령줄 오류 2(clap) 초안
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let lang = Lang::detect();
    let cli = Cli::from_arg_matches(&args::command(lang).get_matches())
        .unwrap_or_else(|error| error.exit());
    let mode = cli.open_mode(lang).unwrap_or_else(|error| error.exit());
    launch::ensure_not_nested(lang)?;
    let chat = commands::chat::resolve_chat(lang, mode)?;
    let add_dirs = commands::chat::resolve_add_dirs(lang, &cli.add_dir)?;
    let mut client = launch::connect_or_start(lang).await?;

    match cli.command {
        None => commands::chat::run(lang, &mut client, chat, &cli.config, add_dirs).await,
        Some(Command::Prune(args)) => commands::prune::run(lang, &mut client, &args).await,
        Some(Command::Export(args)) => commands::export::run(lang, &mut client, &args).await,
        Some(Command::Router {
            command: RouterCommand::Train(args),
        }) => commands::train::run(lang, &mut client, &args).await,
        Some(Command::Router {
            command: RouterCommand::Use(args),
        }) => commands::router::use_version(lang, &mut client, &args).await,
        Some(Command::Router {
            command: RouterCommand::List,
        }) => commands::router::list(lang, &mut client).await,
        Some(Command::Usage(args)) => commands::usage::run(lang, &mut client, &args).await,
    }
}
