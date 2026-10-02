//! `saturn router`: router 버전 관리.
//! 설계: docs/design/router-training.md

use std::io::Write;

use saturn_protocol::rpc::{Notification, Request, RouterVersionInfo};
use saturn_tui::client::EngineClient;
use saturn_tui::i18n::{self, Lang};

use crate::args::RouterUseArgs;
use crate::commands::{call, confirm_on_terminal};

// cost: time O(v), heap O(v), stack O(1), io 2
// vars: v = router 버전 수
// basis: estimate
/// 버전 목록을 한 줄에 하나씩 보인다. 현재 버전 앞에는 `*`를 붙인다.
///
/// # Errors
/// 연결이 끊기거나 engine이 목록 없이 답하면 오류.
pub(crate) async fn list(lang: Lang, client: &mut EngineClient) -> anyhow::Result<()> {
    let mut out = std::io::stdout().lock();
    let (current, versions) = list_versions(lang, client).await?;
    write_versions(&mut out, &current, &versions)
}

fn write_versions(
    out: &mut impl Write,
    current: &str,
    versions: &[RouterVersionInfo],
) -> anyhow::Result<()> {
    for info in versions {
        let marker = if info.version == current { '*' } else { ' ' };
        let ece = info
            .ece
            .map_or_else(|| "-".to_owned(), |ece| format!("{ece:.3}"));
        writeln!(out, "{marker} {} {} ece {ece}", info.version, info.router)?;
    }
    Ok(())
}

// cost: time O(v), heap O(v), stack O(1), io 4
// vars: v = router 버전 수
// basis: estimate
/// 확인 한 줄은 터미널에서만 받는다. 터미널이 없으면 바꾸지 않고 오류로 끝낸다(TODO(#57)).
///
/// # Errors
/// 버전이 없거나 사용자가 확인하지 않았거나 연결이 끊기면 오류.
pub(crate) async fn use_version(
    lang: Lang,
    client: &mut EngineClient,
    args: &RouterUseArgs,
) -> anyhow::Result<()> {
    let mut out = std::io::stdout().lock();
    if args.yes {
        switch_version(lang, client, args, |_| Ok(true), &mut out).await
    } else {
        let confirm = |prompt: &str| confirm_on_terminal(lang, prompt);
        switch_version(lang, client, args, confirm, &mut out).await
    }
}

// cost: time O(v), heap O(v), stack O(1), io 4
// vars: v = router 버전 수
// basis: estimate
async fn switch_version(
    lang: Lang,
    client: &mut EngineClient,
    args: &RouterUseArgs,
    confirm: impl FnOnce(&str) -> anyhow::Result<bool>,
    out: &mut impl Write,
) -> anyhow::Result<()> {
    let (current, versions) = list_versions(lang, client).await?;
    anyhow::ensure!(
        versions.iter().any(|info| info.version == args.version),
        lang.tr(i18n::CLI_ROUTER_VERSION_NOT_FOUND)
            .replace("{version}", &args.version)
            .replace(
                "{available}",
                &versions
                    .iter()
                    .map(|info| info.version.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
    );
    if current == args.version {
        let line = lang
            .tr(i18n::CLI_ROUTER_VERSION_ALREADY)
            .replace("{version}", &current);
        writeln!(out, "{line}")?;
        return Ok(());
    }
    let prompt = lang
        .tr(i18n::CLI_ROUTER_VERSION_PROMPT)
        .replace("{version}", &args.version)
        .replace("{current}", &current);
    if !confirm(&prompt)? {
        anyhow::bail!(
            lang.tr(i18n::CLI_ROUTER_VERSION_NOT_CONFIRMED)
                .replace("{current}", &current)
        );
    }
    let request = Request::UseRouterVersion {
        version: args.version.clone(),
    };
    call(lang, client, request, drop).await?;
    let line = lang
        .tr(i18n::CLI_ROUTER_VERSION_NOW)
        .replace("{version}", &args.version);
    writeln!(out, "{line}")?;
    Ok(())
}

// cost: time O(v), heap O(v), stack O(1), io 2
// vars: v = router 버전 수
// basis: estimate
async fn list_versions(
    lang: Lang,
    client: &mut EngineClient,
) -> anyhow::Result<(String, Vec<RouterVersionInfo>)> {
    let mut listed = None;
    call(lang, client, Request::ListRouterVersions, |notification| {
        if let Notification::RouterVersions { current, versions } = notification {
            listed = Some((current, versions));
        }
    })
    .await?;
    listed.ok_or_else(|| anyhow::anyhow!(lang.tr(i18n::CLI_NO_ROUTER_VERSIONS)))
}

#[cfg(test)]
mod tests {
    use crate::testing::{FakeEngine, Reply};

    use super::*;

    fn info(version: &str) -> RouterVersionInfo {
        RouterVersionInfo {
            version: version.to_owned(),
            router: "jev".to_owned(),
            ece: None,
            questions: Vec::new(),
        }
    }

    fn versions(current: &str) -> Reply {
        Reply::with(vec![Notification::RouterVersions {
            current: current.to_owned(),
            versions: vec![info("v1"), info("v2")],
        }])
    }

    fn args(version: &str) -> RouterUseArgs {
        RouterUseArgs {
            version: version.to_owned(),
            yes: false,
        }
    }

    #[tokio::test]
    async fn switch_version_after_confirmation_uses_version() {
        let engine = FakeEngine::start(vec![versions("v1"), Reply::ok()]);
        let mut client = engine.client().await;
        let mut out = Vec::new();

        switch_version(Lang::En, &mut client, &args("v2"), |_| Ok(true), &mut out)
            .await
            .unwrap();

        assert_eq!(
            engine.finish().await,
            vec![
                Request::ListRouterVersions,
                Request::UseRouterVersion {
                    version: "v2".to_owned()
                }
            ]
        );
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "now using router version v2\n"
        );
    }

    #[tokio::test]
    async fn switch_version_without_confirmation_changes_nothing() {
        let engine = FakeEngine::start(vec![versions("v1")]);
        let mut client = engine.client().await;

        let result = switch_version(
            Lang::En,
            &mut client,
            &args("v2"),
            |_| Ok(false),
            &mut Vec::new(),
        )
        .await;

        assert!(result.is_err());
        assert_eq!(engine.finish().await, vec![Request::ListRouterVersions]);
    }

    #[tokio::test]
    async fn switch_version_unknown_version_lists_available() {
        let engine = FakeEngine::start(vec![versions("v1")]);
        let mut client = engine.client().await;

        let error = switch_version(
            Lang::En,
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
            Lang::En,
            &mut client,
            &args("v2"),
            |_| panic!("must not ask"),
            &mut out,
        )
        .await
        .unwrap();

        assert_eq!(
            String::from_utf8(out).unwrap(),
            "already using router version v2\n"
        );
    }

    #[test]
    fn write_versions_marks_current_and_shows_ece() {
        let mut v2 = info("v2");
        v2.ece = Some(0.0421);
        let mut out = Vec::new();

        write_versions(&mut out, "v2", &[info("v1"), v2]).unwrap();

        let text = String::from_utf8(out).unwrap();
        let lines: Vec<_> = text.lines().collect();
        assert!(lines[0].starts_with("  v1 "));
        assert!(lines[0].ends_with("ece -"));
        assert!(lines[1].starts_with("* v2 "));
        assert!(lines[1].ends_with("ece 0.042"));
    }
}
