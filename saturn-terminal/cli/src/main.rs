//! `saturn` 실행 파일: 명령 해석, engine 띄우기·붙기, TUI 실행, plain 모드.
//!
//! 설계: docs/architecture.md(프로세스 배치), docs/design/engine-lifecycle.md(engine 시작).
//! cli는 engine 라이브러리에 링크하지 않는다. `saturn-engine` 실행 파일을 띄우고 소켓으로 붙는다.

// TODO(#74): 뼈대 단계라 본문이 `todo!`인 함수의 인자가 쓰이지 않는다. 구현 이슈가 모두 닫히면 이 허용을 지운다
#![allow(unused_variables, dead_code)]

use clap::Parser;

use crate::args::{Cli, Command, JudgeCommand};

mod args;
mod commands;
mod launch;

// TODO(#47): 종료 코드. 지금은 `anyhow` 기본(실패 1)만 쓴다
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
