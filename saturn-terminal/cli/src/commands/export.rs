//! `saturn export`: 판단 기록 JSONL 내보내기.
//! 설계: docs/design/records.md

use std::io::Write;
use std::path::PathBuf;

use anyhow::Context;
use saturn_protocol::rpc::Request;
use saturn_tui::client::EngineClient;
use saturn_tui::i18n::{self, Lang};

use crate::args::ExportArgs;
use crate::commands::call;

// cost: time O(1), heap O(1), stack O(1), io 2
// basis: estimate
/// 경로는 절대 경로로 바꿔 보낸다. engine과 작업 폴더가 다를 수 있기 때문이다.
///
/// # Errors
/// 경로를 만들 수 없거나 engine이 쓰지 못하면 오류.
pub(crate) async fn run(
    lang: Lang,
    client: &mut EngineClient,
    args: &ExportArgs,
) -> anyhow::Result<()> {
    let path = export(lang, client, args).await?;
    let line = lang
        .tr(i18n::CLI_EXPORTED)
        .replace("{path}", &path.display().to_string());
    writeln!(std::io::stdout().lock(), "{line}")?;
    Ok(())
}

// cost: time O(1), heap O(1), stack O(1), io 1
// basis: estimate
async fn export(
    lang: Lang,
    client: &mut EngineClient,
    args: &ExportArgs,
) -> anyhow::Result<PathBuf> {
    let path = std::path::absolute(&args.path).with_context(|| {
        lang.tr(i18n::CLI_PATH_UNRESOLVED)
            .replace("{path}", &args.path.display().to_string())
    })?;
    let request = Request::ExportJudgments {
        path: path.display().to_string(),
    };
    call(lang, client, request, drop).await?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use crate::testing::{FakeEngine, Reply};

    use super::*;

    #[tokio::test]
    async fn export_sends_absolute_path() {
        let engine = FakeEngine::start(vec![Reply::ok()]);
        let mut client = engine.client().await;
        let args = ExportArgs {
            path: PathBuf::from("out/judgments.jsonl"),
        };

        let path = export(Lang::En, &mut client, &args).await.unwrap();

        let expected = std::env::current_dir().unwrap().join("out/judgments.jsonl");
        assert_eq!(path, expected);
        assert_eq!(
            engine.finish().await,
            vec![Request::ExportJudgments {
                path: expected.display().to_string()
            }]
        );
    }

    #[tokio::test]
    async fn export_keeps_absolute_path_as_is() {
        let engine = FakeEngine::start(vec![Reply::ok()]);
        let mut client = engine.client().await;
        let args = ExportArgs {
            path: PathBuf::from("/data/judgments.jsonl"),
        };

        let path = export(Lang::En, &mut client, &args).await.unwrap();

        assert_eq!(path, PathBuf::from("/data/judgments.jsonl"));
    }

    #[tokio::test]
    async fn export_when_engine_rejects_is_error() {
        let engine = FakeEngine::start(vec![Reply::error(-32603, "failed to write")]);
        let mut client = engine.client().await;
        let args = ExportArgs {
            path: PathBuf::from("/data/judgments.jsonl"),
        };

        let error = export(Lang::En, &mut client, &args).await.unwrap_err();

        assert!(error.to_string().contains("failed to write"));
    }
}
