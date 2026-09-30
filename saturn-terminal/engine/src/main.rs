//! `saturn-engine` 실행 파일. cli(`saturn`)가 engine이 없을 때 띄운다.
//!
//! 설계: docs/design/engine-lifecycle.md(engine 시작 순서, 프로세스 배치와 수명).
//! 인자: `--home <폴더>`(기본 `$HOME/.saturn`), `-c key=value`(여러 번). 작업 폴더는 띄운 위치다.
//! 로그는 stderr이고 레벨은 `RUST_LOG`로 정한다.

use std::path::PathBuf;

use anyhow::Context;
use saturn_engine::{Engine, EngineOptions};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let options = parse_options(std::env::args().skip(1))?;
    Engine::run(options).await.context("engine stopped")
}

/// 명령 인자를 읽는다. 모르는 인자는 오류다.
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
    let workdir = std::env::current_dir().context("failed to read current directory")?;
    Ok(EngineOptions {
        home,
        workdir,
        run_overrides,
    })
}

/// `$HOME/.saturn`. TODO(#49): 경로 설정 키
fn default_home() -> anyhow::Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME should be set")?;
    Ok(PathBuf::from(home).join(".saturn"))
}
