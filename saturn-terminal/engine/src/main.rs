//! `saturn-engine` 실행 파일. engine이 없을 때 `saturn`이 띄운다.
//! 설계: docs/design/engine-lifecycle.md

use std::path::PathBuf;

use anyhow::Context;
use saturn_engine::engine_log::EngineLog;
use saturn_engine::{Engine, EngineOptions};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let options = parse_options(std::env::args().skip(1))?;
    let log = EngineLog::start(&options.home).context("failed to start engine log")?;
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(log)
        .init();
    // cli가 engine의 stderr를 버리므로 panic과 종료 사유는 로그 파일에만 남긴다
    std::panic::set_hook(Box::new(|info| tracing::error!(%info, "engine panicked")));

    let result = Engine::run(options).await.context("engine stopped");
    if let Err(error) = &result {
        tracing::error!("{error:#}");
    }
    result
}

fn parse_options(args: impl Iterator<Item = String>) -> anyhow::Result<EngineOptions> {
    let mut home = None;
    let mut run_overrides = Vec::new();
    let mut args = args;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--home" => home = Some(PathBuf::from(args.next().context("--home needs a path")?)),
            "-c" => run_overrides.push(args.next().context("-c needs key=value")?),
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }
    let home = match home {
        Some(home) => home,
        None => default_home()?,
    };
    Ok(EngineOptions {
        home,
        run_overrides,
    })
}

/// TODO(#235): 경로 설정 키
fn default_home() -> anyhow::Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME should be set")?;
    Ok(PathBuf::from(home).join(".saturn"))
}
