//! `saturn` 실행 파일. engine 라이브러리에 링크하지 않고 `saturn-engine`을 띄워 소켓으로 붙는다.
//! 설계: docs/architecture.md

// TODO(#74): `todo!` 뼈대의 미사용 인자 허용. 구현 이슈가 모두 닫히면 지운다
#![allow(unused_variables, dead_code)]

use clap::Parser;

use crate::args::{Cli, Command, JudgeCommand};

mod args;
mod commands;
mod launch;

// TODO(#47): 종료 코드. 지금은 `anyhow` 기본(실패 1)
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    launch::ensure_not_nested()?;
    let mut client = launch::connect_or_start().await?;

    match cli.command {
        None => commands::chat::run(&mut client, &cli.config).await,
        Some(Command::Train(args)) => commands::train::run(&mut client, &args).await,
        Some(Command::Prune(args)) => commands::prune::run(&mut client, &args).await,
        Some(Command::Export(args)) => commands::export::run(&mut client, &args).await,
        Some(Command::Judge {
            command: JudgeCommand::Version(args),
        }) => commands::judge::use_version(&mut client, &args).await,
        Some(Command::Usage(args)) => commands::usage::run(&mut client, &args).await,
    }
}
