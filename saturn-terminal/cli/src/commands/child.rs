//! 에이전트 작업 안의 `saturn`: 떠 있는 engine에 출입증으로 접속해 부모 채팅의 하위 작업으로 일을 맡긴다.
//! 새 engine을 띄우지 않고, 표준 입력의 줄을 입력으로 보내 결과 줄을 표준 출력에 쓴다.
//! 설계: docs/design/child-sessions.md

use std::path::Path;

use anyhow::Context;
use saturn_tui::client::EngineClient;
use saturn_tui::i18n::{self, Lang};
use saturn_tui::{ChildAccess, RunOptions};

use crate::args::Cli;
use crate::exit::{Exit, ExitCode};

// cost: time O(c), heap O(c), stack O(1), io c
// vars: c = 오간 메시지 수
// basis: estimate
/// 작업 폴더, 더한 폴더, 환경, 실행 층은 engine이 부모에게서 물려주므로 받지 않는다.
///
/// # Errors
/// 하위 접속에 쓸 수 없는 인자가 있거나, engine이 떠 있지 않거나, engine이 거절하면 오류.
pub(crate) async fn run(lang: Lang, cli: &Cli, pass: String, socket: &Path) -> anyhow::Result<()> {
    if cli.command.is_some()
        || !cli.config.is_empty()
        || !cli.add_dir.is_empty()
        || cli.continue_last
        || cli.resume.is_some()
    {
        return Err(Exit::error(ExitCode::Usage, lang.tr(i18n::CLI_CHILD_ARGS)));
    }
    let mut client = EngineClient::connect(socket)
        .await
        .context(lang.tr(i18n::CLI_CHILD_NO_ENGINE))
        .map_err(|error| Exit::wrap(ExitCode::EngineUnavailable, error))?;
    let options = RunOptions {
        chat: None,
        lang: Some(lang),
        workdir: std::env::current_dir().context(lang.tr(i18n::CLI_CURRENT_DIR_UNREADABLE))?,
        overrides: Vec::new(),
        add_dirs: Vec::new(),
        history: std::path::PathBuf::new(),
        child: Some(ChildAccess {
            pass,
            mode: cli.mode.clone(),
        }),
    };
    Ok(saturn_tui::run_plain(&mut client, options).await?)
}
