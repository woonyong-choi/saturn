//! `saturn judge`: judge 버전 관리.
//! 설계: docs/design/judge-training.md

use std::io::Write;

use saturn_protocol::rpc::{JudgeVersionInfo, Notification, Request};
use saturn_tui::client::EngineClient;

use crate::args::JudgeVersionArgs;
use crate::commands::{call, confirm_on_terminal};

// cost: time O(v), heap O(v), stack O(1), io 4
// vars: v = judge 버전 수
// basis: estimate
/// 확인 한 줄은 터미널에서만 받는다. 터미널이 없으면 바꾸지 않고 오류로 끝낸다(TODO(#57)).
///
/// # Errors
/// 버전이 없거나 사용자가 확인하지 않았거나 연결이 끊기면 오류.
pub(crate) async fn use_version(
    client: &mut EngineClient,
    args: &JudgeVersionArgs,
) -> anyhow::Result<()> {
    let mut out = std::io::stdout().lock();
    switch_version(client, args, confirm_on_terminal, &mut out).await
}

// cost: time O(v), heap O(v), stack O(1), io 4
// vars: v = judge 버전 수
// basis: estimate
async fn switch_version(
    client: &mut EngineClient,
    args: &JudgeVersionArgs,
    confirm: impl FnOnce(&str) -> anyhow::Result<bool>,
    out: &mut impl Write,
) -> anyhow::Result<()> {
    let (current, versions) = list_versions(client).await?;
    anyhow::ensure!(
        versions.iter().any(|info| info.version == args.version),
        "judge version not found: {} (available: {})",
        args.version,
        versions
            .iter()
            .map(|info| info.version.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    if current == args.version {
        writeln!(out, "already using judge version {current}")?;
        return Ok(());
    }
    let prompt = format!("use judge version {} (current: {current})?", args.version);
    if !confirm(&prompt)? {
        anyhow::bail!("not confirmed; judge version stays {current}");
    }
    let request = Request::UseJudgeVersion {
        version: args.version.clone(),
    };
    call(client, request, drop).await?;
    writeln!(out, "now using judge version {}", args.version)?;
    Ok(())
}

// cost: time O(v), heap O(v), stack O(1), io 2
// vars: v = judge 버전 수
// basis: estimate
async fn list_versions(
    client: &mut EngineClient,
) -> anyhow::Result<(String, Vec<JudgeVersionInfo>)> {
    let mut listed = None;
    call(client, Request::ListJudgeVersions, |notification| {
        if let Notification::JudgeVersions { current, versions } = notification {
            listed = Some((current, versions));
        }
    })
    .await?;
    listed.ok_or_else(|| anyhow::anyhow!("engine answered without judge versions"))
}

#[cfg(test)]
mod tests {
    use crate::testing::{FakeEngine, Reply};

    use super::*;

    fn info(version: &str) -> JudgeVersionInfo {
        JudgeVersionInfo {
            version: version.to_owned(),
            judge: "jev".to_owned(),
            ece: None,
            questions: Vec::new(),
        }
    }

    fn versions(current: &str) -> Reply {
        Reply::with(vec![Notification::JudgeVersions {
            current: current.to_owned(),
            versions: vec![info("v1"), info("v2")],
        }])
    }

    fn args(version: &str) -> JudgeVersionArgs {
        JudgeVersionArgs {
            version: version.to_owned(),
        }
    }

    #[tokio::test]
    async fn switch_version_after_confirmation_uses_version() {
        let engine = FakeEngine::start(vec![versions("v1"), Reply::ok()]);
        let mut client = engine.client().await;
        let mut out = Vec::new();

        switch_version(&mut client, &args("v2"), |_| Ok(true), &mut out)
            .await
            .unwrap();

        assert_eq!(
            engine.finish().await,
            vec![
                Request::ListJudgeVersions,
                Request::UseJudgeVersion {
                    version: "v2".to_owned()
                }
            ]
        );
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "now using judge version v2\n"
        );
    }

    #[tokio::test]
    async fn switch_version_without_confirmation_changes_nothing() {
        let engine = FakeEngine::start(vec![versions("v1")]);
        let mut client = engine.client().await;

        let result = switch_version(&mut client, &args("v2"), |_| Ok(false), &mut Vec::new()).await;

        assert!(result.is_err());
        assert_eq!(engine.finish().await, vec![Request::ListJudgeVersions]);
    }

    #[tokio::test]
    async fn switch_version_unknown_version_lists_available() {
        let engine = FakeEngine::start(vec![versions("v1")]);
        let mut client = engine.client().await;

        let error = switch_version(
            &mut client,
            &args("v9"),
            |_| panic!("must not ask"),
            &mut Vec::new(),
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("available: v1, v2"));
    }

    #[tokio::test]
    async fn switch_version_to_current_asks_nothing() {
        let engine = FakeEngine::start(vec![versions("v2")]);
        let mut client = engine.client().await;
        let mut out = Vec::new();

        switch_version(
            &mut client,
            &args("v2"),
            |_| panic!("must not ask"),
            &mut out,
        )
        .await
        .unwrap();

        assert_eq!(
            String::from_utf8(out).unwrap(),
            "already using judge version v2\n"
        );
    }
}
