//! `saturn prune`: 기록 정리.
//! 설계: docs/design/records.md

use std::io::Write;

use saturn_protocol::rpc::Request;
use saturn_tui::client::EngineClient;
use saturn_tui::i18n::{self, Lang};

use crate::args::PruneArgs;
use crate::commands::call;

// cost: time O(1), heap O(1), stack O(1), io 2
// basis: estimate
/// 삭제 제외 대상(열린 입력, 활성 session, 판단 기록 등)은 engine이 거른다.
///
/// TODO(#161): 미리보기 대상 목록을 돌려받는 알림이 생기면 `--yes` 없는 실행에서 그 목록을 보인다
///
/// # Errors
/// engine이 거절했거나 연결이 끊기면 오류.
pub(crate) async fn run(
    lang: Lang,
    client: &mut EngineClient,
    args: &PruneArgs,
) -> anyhow::Result<()> {
    call(lang, client, Request::Prune { yes: args.yes }, drop).await?;
    let message = if args.yes {
        i18n::CLI_PRUNED
    } else {
        i18n::CLI_PRUNE_PREVIEW
    };
    writeln!(std::io::stdout().lock(), "{}", lang.tr(message))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::testing::{FakeEngine, Reply};

    use super::*;

    #[tokio::test]
    async fn run_without_yes_asks_for_preview_only() {
        let engine = FakeEngine::start(vec![Reply::ok()]);
        let mut client = engine.client().await;

        run(Lang::En, &mut client, &PruneArgs { yes: false })
            .await
            .unwrap();

        assert_eq!(engine.finish().await, vec![Request::Prune { yes: false }]);
    }

    #[tokio::test]
    async fn run_with_yes_deletes() {
        let engine = FakeEngine::start(vec![Reply::ok()]);
        let mut client = engine.client().await;

        run(Lang::En, &mut client, &PruneArgs { yes: true })
            .await
            .unwrap();

        assert_eq!(engine.finish().await, vec![Request::Prune { yes: true }]);
    }

    #[tokio::test]
    async fn run_when_engine_rejects_is_error() {
        let engine = FakeEngine::start(vec![Reply::error(-32601, "unsupported: Prune")]);
        let mut client = engine.client().await;

        let result = run(Lang::En, &mut client, &PruneArgs { yes: true }).await;

        assert!(result.is_err());
    }
}
