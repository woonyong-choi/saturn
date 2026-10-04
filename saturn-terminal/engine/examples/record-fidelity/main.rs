//! 기록 충실도 실험의 변환 도구. engine의 실제 provider 연결로 작업을 한 번 진행하거나, 저장한 원시 줄을 같은 연결 코드로 다시 읽어 Saturn 이벤트를 JSONL로 낸다. engine에 붙지 않는다.
//! 실행: `cargo run -q -p saturn-engine --example record-fidelity -- <인자>`
//! 설계: docs/experiments/record-fidelity/design.md

use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, bail};
use saturn_core::providers::{ProviderClient, SessionSpec};
use saturn_engine::{
    LaunchSpec, Masker, PermissionLaunch, Registry, SaturnDefaults, Supervisor, UserProviderConfig,
};
use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::{AgentId, Provider, SettingsRevision};
use saturn_protocol::rpc::PermissionAnswer;

const USAGE: &str = "usage: record-fidelity <provider-id> --program FILE --workdir DIR --prompt-file FILE --out FILE [--model NAME] [--timeout-s N]";

#[derive(Debug)]
struct Args {
    provider: Provider,
    program: PathBuf,
    workdir: PathBuf,
    prompt_file: PathBuf,
    out: PathBuf,
    model: Option<String>,
    timeout: Duration,
}

// cost: time O(a), heap O(a), stack O(1)
// vars: a = 인자 수
// basis: estimate
/// # Errors
/// 인자가 모자라거나 모르는 값이면 오류.
fn parse_args(raw: &[String]) -> anyhow::Result<Args> {
    let Some(provider) = raw.first().and_then(|id| Provider::parse(id).ok()) else {
        bail!("{USAGE}");
    };
    let value = |name: &str| {
        raw.iter()
            .position(|arg| arg == name)
            .and_then(|index| raw.get(index + 1))
    };
    let required = |name: &str| value(name).with_context(|| format!("missing {name}\n{USAGE}"));
    let timeout_s: u64 = match value("--timeout-s") {
        Some(text) => text.parse().context("invalid --timeout-s")?,
        None => 600,
    };
    Ok(Args {
        provider,
        program: required("--program")?.into(),
        workdir: required("--workdir")?.into(),
        prompt_file: required("--prompt-file")?.into(),
        out: required("--out")?.into(),
        model: value("--model").cloned(),
        timeout: Duration::from_secs(timeout_s),
    })
}

fn launch_spec(args: &Args) -> LaunchSpec {
    let env: Vec<(OsString, OsString)> = std::env::vars_os().collect();
    LaunchSpec {
        provider: args.provider,
        program: args.program.clone(),
        workdir: args.workdir.clone(),
        settings: SettingsRevision(1),
        user_config: UserProviderConfig {
            has_auto_compact: true,
        },
        defaults: SaturnDefaults {
            auto_compact_tokens: Some(180_000),
        },
        env,
        hook_settings: None,
        key_deny_read: Vec::new(),
        permission: PermissionLaunch::default(),
        masker: Masker::new(Vec::new()),
    }
}

// cost: time O(e), heap O(1), stack O(1), io e
// vars: e = 받은 이벤트 수
// basis: estimate
/// 첫 턴이 끝나거나 연결이 끊기거나 시간이 다하면 멈춘다. 이벤트는 받은 순서대로 한 줄씩 쓴다.
///
/// # Errors
/// 연결, 전송, 출력 쓰기가 실패하면 오류. 시간 초과는 오류가 아니라 `timeout` 줄을 남긴다.
async fn run(args: &Args) -> anyhow::Result<()> {
    let prompt = std::fs::read_to_string(&args.prompt_file).context("failed to read prompt")?;
    let mut out = std::fs::File::create(&args.out).context("failed to create output")?;
    let mut connection = Registry::builtin()
        .connect(launch_spec(args), Supervisor::new())
        .await
        .map_err(|error| anyhow::anyhow!("failed to connect: {error}"))?;
    let handle = connection
        .open_session(SessionSpec {
            agent: AgentId(1),
            workdir: args.workdir.clone(),
            model: args.model.clone(),
            settings: SettingsRevision(1),
            resume: None,
            packet: None,
            add_dirs: Vec::new(),
            interrupted_children: Vec::new(),
        })
        .await
        .map_err(|error| anyhow::anyhow!("failed to open session: {error}"))?;
    connection
        .send_turn(&handle.provider_session, &prompt)
        .await
        .map_err(|error| anyhow::anyhow!("failed to send turn: {error}"))?;
    let deadline = tokio::time::Instant::now() + args.timeout;
    loop {
        let next = tokio::time::timeout_at(deadline, connection.next_event()).await;
        let Ok(event) = next else {
            writeln!(out, "{{\"timeout\":true}}")?;
            break;
        };
        let Some(event) = event else {
            break;
        };
        writeln!(out, "{}", serde_json::to_string(&event)?)?;
        if let ProviderEvent::PermissionRequested { request_id, .. } = &event {
            // 이 도구는 사람이 답하지 않으므로 모든 승인 요청을 허용해 작업이 끝까지 돌게 한다
            connection
                .answer_permission(
                    &handle.provider_session,
                    request_id,
                    PermissionAnswer::AllowOnce,
                )
                .await
                .map_err(|error| anyhow::anyhow!("failed to answer permission: {error}"))?;
        }
        if matches!(
            event,
            ProviderEvent::TurnCompleted { .. } | ProviderEvent::StreamLost { .. }
        ) {
            break;
        }
    }
    let _ = connection.close_session(&handle.provider_session).await; // 이미 끝난 session일 수 있다
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();
    let raw: Vec<String> = std::env::args().skip(1).collect();
    run(&parse_args(&raw)?).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| (*part).to_owned()).collect()
    }

    #[test]
    fn parse_args_reads_required_and_optional_values() {
        let raw = strings(&[
            "codex",
            "--program",
            "p",
            "--workdir",
            "w",
            "--prompt-file",
            "f",
            "--out",
            "o",
            "--model",
            "m",
        ]);

        let args = parse_args(&raw).unwrap();

        assert_eq!(args.provider, Provider::from_static("codex"));
        assert_eq!(args.model.as_deref(), Some("m"));
        assert_eq!(args.timeout, Duration::from_secs(600));
    }

    #[test]
    fn parse_args_missing_value_returns_error() {
        let error = parse_args(&strings(&["claude", "--program", "p"])).unwrap_err();

        assert!(error.to_string().starts_with("missing --workdir"));
    }
}
