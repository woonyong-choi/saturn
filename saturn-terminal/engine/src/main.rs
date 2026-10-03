//! `saturn-engine` 실행 파일. engine이 없을 때 `saturn`이 띄운다.
//! 설계: docs/design/engine-lifecycle.md

use std::io::{Read, Write};
use std::path::PathBuf;

use anyhow::Context;
use saturn_engine::engine_log::EngineLog;
use saturn_engine::{Engine, EngineOptions, HookInputError, run_pre_tool_use};

/// provider 훅이 실행 파일에 거는 하위 명령. 잠금과 소켓을 열지 않고 바로 끝난다.
const HOOK_COMMAND: &str = "hook";

const PRE_TOOL_USE: &str = "pre-tool-use";

/// 훅 입력을 읽지 못했을 때 도구 호출을 막는 Claude Code 훅 종료 코드.
const BLOCKING_EXIT_CODE: u8 = 2;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1).peekable();
    if args.peek().is_some_and(|arg| arg == HOOK_COMMAND) {
        return run_hook(args.skip(1));
    }
    let options = parse_options(args)?;
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

/// 판정 결과는 stdout JSON이고 종료 코드는 0이다. 입력이 잘못되면 stderr에 이유를 쓰고 2로 끝난다.
fn run_hook(mut args: impl Iterator<Item = String>) -> anyhow::Result<()> {
    let name = args.next().context("hook needs a name")?;
    anyhow::ensure!(name == PRE_TOOL_USE, "unknown hook: {name}");
    let mut saturn_home = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--home" => {
                saturn_home = Some(PathBuf::from(args.next().context("--home needs a path")?));
            }
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }
    let user_home = PathBuf::from(std::env::var_os("HOME").context("HOME should be set")?);
    let saturn_home = saturn_home.unwrap_or_else(|| user_home.join(".saturn"));
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .context("failed to read hook input")?;
    match run_pre_tool_use(&saturn_home, &user_home, &input) {
        Ok(Some(output)) => {
            writeln!(std::io::stdout(), "{output}").context("failed to write hook output")?;
        }
        Ok(None) => {}
        Err(error @ (HookInputError::NotJson | HookInputError::NoToolName)) => {
            // 쓰기에 실패해도 종료 코드 2로 막는 동작은 같다
            let _ = writeln!(std::io::stderr(), "{error}");
            std::process::exit(i32::from(BLOCKING_EXIT_CODE));
        }
    }
    Ok(())
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
