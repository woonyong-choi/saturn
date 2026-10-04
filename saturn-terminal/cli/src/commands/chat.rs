//! 하위 명령 없는 `saturn`: 대화 화면을 연다.
//! 설계: docs/design/tui.md

use std::io::IsTerminal;
use std::path::PathBuf;

use anyhow::Context;
use saturn_protocol::ids::ChatId;
use saturn_protocol::rpc::{QueryResult, Request};
use saturn_tui::RunOptions;
use saturn_tui::client::EngineClient;
use saturn_tui::i18n::{self, Lang};

use crate::args::{ConfigOverride, OpenMode};
use crate::commands::{call, resume};
use crate::exit::{Exit, ExitCode};

const HISTORY_FILE: &str = "history";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScreenMode {
    FullScreen,
    /// 파이프와 CI용. 같은 명령은 전체 화면과 같은 결과를 낸다.
    Plain,
}

/// engine에 붙기 전에 불러, 목록에서 고르는 방식인데 터미널이 없으면 engine을 띄우지 않는다.
///
/// # Errors
/// 채팅 id 없이 목록에서 고르는 방식인데 표준 입력이 터미널이 아니면 오류.
pub(crate) fn ensure_can_open(lang: Lang, mode: OpenMode) -> anyhow::Result<()> {
    match mode {
        OpenMode::PickInFolder | OpenMode::PickInAll => resume::ensure_terminal(lang),
        OpenMode::New | OpenMode::Chat(_) | OpenMode::ContinueLast => Ok(()),
    }
}

// cost: time O(1), heap O(1), stack O(1), io 2
// basis: estimate
/// 이어 열 채팅을 정한다. `--continue`는 지금 폴더의 최근 채팅을 engine에 묻고, `--resume`은 목록에서 고른다.
///
/// # Errors
/// `--continue`인데 폴더에 채팅이 없거나, 목록에서 고르지 않았거나, 연결이 끊기면 오류. 새 채팅은 열지 않는다.
pub(crate) async fn resolve_chat(
    lang: Lang,
    client: &mut EngineClient,
    mode: OpenMode,
) -> anyhow::Result<Option<ChatId>> {
    match mode {
        OpenMode::New => Ok(None),
        OpenMode::Chat(chat) => Ok(Some(chat)),
        OpenMode::ContinueLast => latest_chat(lang, client).await.map(Some),
        OpenMode::PickInFolder => {
            let folder = current_folder(lang)?;
            resume::pick(lang, client, Some(folder)).await.map(Some)
        }
        OpenMode::PickInAll => resume::pick(lang, client, None).await.map(Some),
    }
}

fn current_folder(lang: Lang) -> anyhow::Result<String> {
    Ok(std::env::current_dir()
        .context(lang.tr(i18n::CLI_CURRENT_DIR_UNREADABLE))?
        .display()
        .to_string())
}

async fn latest_chat(lang: Lang, client: &mut EngineClient) -> anyhow::Result<ChatId> {
    let folder = current_folder(lang)?;
    let Some(QueryResult::LatestChat { chat }) =
        call(lang, client, Request::LatestChat { folder }, drop).await?
    else {
        return Err(Exit::error(
            ExitCode::EngineInternal,
            lang.tr(i18n::CLI_NO_LATEST_CHAT_ANSWER),
        ));
    };
    chat.ok_or_else(|| Exit::error(ExitCode::NotFound, lang.tr(i18n::CLI_NO_CHAT_TO_CONTINUE)))
}

// cost: time O(d·p), heap O(d·p), stack O(1), io d·p
// vars: d = `--add-dir` 개수, p = 경로 깊이
// basis: estimate
/// `--add-dir` 폴더를 링크를 푼 절대 경로로 바꾼다. engine에 붙기 전에 불러 없는 폴더면 engine을 띄우지 않는다.
///
/// # Errors
/// 폴더가 없거나 폴더가 아니면 오류.
pub(crate) fn resolve_add_dirs(lang: Lang, dirs: &[PathBuf]) -> anyhow::Result<Vec<PathBuf>> {
    dirs.iter()
        .map(|dir| {
            let shown = dir.display().to_string();
            let resolved = std::fs::canonicalize(dir).map_err(|error| {
                let code = if error.kind() == std::io::ErrorKind::NotFound {
                    ExitCode::NotFound
                } else {
                    ExitCode::Failure
                };
                let message = lang
                    .tr(i18n::CLI_ADD_DIR_UNREADABLE)
                    .replace("{dir}", &shown);
                Exit::wrap(code, anyhow::Error::new(error).context(message))
            })?;
            if !resolved.is_dir() {
                return Err(Exit::error(
                    ExitCode::Usage,
                    lang.tr(i18n::CLI_ADD_DIR_NOT_FOLDER)
                        .replace("{dir}", &shown),
                ));
            }
            Ok(resolved)
        })
        .collect()
}

// cost: time O(c), heap O(c), stack O(1), io c
// vars: c = 화면을 연 동안 오간 메시지 수
// basis: estimate
/// # Errors
/// `-c` 값을 engine이 받지 않았거나 연결이 끊기면 오류.
pub(crate) async fn run(
    lang: Lang,
    client: &mut EngineClient,
    chat: Option<ChatId>,
    config: &[ConfigOverride],
    add_dirs: Vec<PathBuf>,
) -> anyhow::Result<()> {
    let workdir = std::env::current_dir().context(lang.tr(i18n::CLI_CURRENT_DIR_UNREADABLE))?;
    let options = run_options(chat, config, add_dirs, workdir);
    match detect_mode() {
        ScreenMode::FullScreen => run_full_screen(client, options).await,
        ScreenMode::Plain => run_plain(client, options).await,
    }
}

/// TODO(#57): plain을 켜는 조건과 우선순위, 설정 키. 지금은 표준 입력이나 표준 출력이 터미널이 아니면 plain
fn detect_mode() -> ScreenMode {
    mode_for(
        std::io::stdin().is_terminal(),
        std::io::stdout().is_terminal(),
    )
}

fn mode_for(stdin_is_terminal: bool, stdout_is_terminal: bool) -> ScreenMode {
    if stdin_is_terminal && stdout_is_terminal {
        ScreenMode::FullScreen
    } else {
        ScreenMode::Plain
    }
}

// cost: time O(c), heap O(c), stack O(1)
// vars: c = `-c` 개수
// basis: estimate
/// `-c`는 실행 층으로 `Attach`에 실려 이 접속의 입력에만 적용된다.
fn run_options(
    chat: Option<ChatId>,
    config: &[ConfigOverride],
    add_dirs: Vec<PathBuf>,
    workdir: PathBuf,
) -> RunOptions {
    let history = EngineClient::default_socket()
        .parent()
        .map(|home| home.join(HISTORY_FILE))
        .unwrap_or_else(|| PathBuf::from(HISTORY_FILE));
    RunOptions {
        chat,
        lang: None,
        workdir,
        overrides: config
            .iter()
            .map(|entry| (entry.key.clone(), entry.value.clone()))
            .collect(),
        add_dirs,
        history,
        child: None,
    }
}

async fn run_full_screen(client: &mut EngineClient, options: RunOptions) -> anyhow::Result<()> {
    Ok(saturn_tui::run(client, options).await?)
}

// cost: time O(c), heap O(c), stack O(1), io c
// vars: c = 오간 메시지 수. `saturn_tui::run_plain`을 부른다
// basis: estimate
async fn run_plain(client: &mut EngineClient, options: RunOptions) -> anyhow::Result<()> {
    Ok(saturn_tui::run_plain(client, options).await?)
}

#[cfg(test)]
mod tests {
    use crate::testing::{FakeEngine, Reply};

    use super::*;

    fn entry(key: &str, value: &str) -> ConfigOverride {
        ConfigOverride {
            key: key.to_owned(),
            value: value.to_owned(),
        }
    }

    #[tokio::test]
    async fn continue_resume_new_chat_has_no_chat_id() {
        let engine = FakeEngine::start(vec![]);
        let mut client = engine.client().await;

        let chat = resolve_chat(Lang::En, &mut client, OpenMode::New).await;

        assert_eq!(chat.unwrap(), None);
        assert!(engine.finish().await.is_empty());
    }

    #[tokio::test]
    async fn continue_resume_chat_id_is_passed_to_attach() {
        let engine = FakeEngine::start(vec![]);
        let mut client = engine.client().await;

        let chat = resolve_chat(Lang::En, &mut client, OpenMode::Chat(ChatId(7))).await;

        assert_eq!(chat.unwrap(), Some(ChatId(7)));
        assert!(engine.finish().await.is_empty());
    }

    #[tokio::test]
    async fn continue_opens_the_latest_chat_of_the_current_folder() {
        let engine = FakeEngine::start(vec![Reply::result(QueryResult::LatestChat {
            chat: Some(ChatId(5)),
        })]);
        let mut client = engine.client().await;

        let chat = resolve_chat(Lang::En, &mut client, OpenMode::ContinueLast).await;

        assert_eq!(chat.unwrap(), Some(ChatId(5)));
        let folder = std::env::current_dir().unwrap().display().to_string();
        let requests = engine.finish().await;
        assert!(matches!(
            requests.as_slice(),
            [Request::LatestChat { folder: asked }] if *asked == folder
        ));
    }

    #[tokio::test]
    async fn continue_without_a_chat_in_the_folder_ends_with_guidance_and_opens_nothing() {
        let engine = FakeEngine::start(vec![Reply::result(QueryResult::LatestChat { chat: None })]);
        let mut client = engine.client().await;

        let error = resolve_chat(Lang::En, &mut client, OpenMode::ContinueLast)
            .await
            .unwrap_err();

        assert!(error.to_string().contains("No chat to continue"));
        assert_eq!(engine.finish().await.len(), 1);
    }

    #[tokio::test]
    async fn resume_in_the_folder_picks_from_the_chats_of_the_current_folder() {
        let engine = FakeEngine::start(vec![Reply::result(QueryResult::Chats {
            chats: Vec::new(),
        })]);
        let mut client = engine.client().await;

        let error = resolve_chat(Lang::En, &mut client, OpenMode::PickInFolder)
            .await
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("No chat to continue in this folder")
        );
        let folder = std::env::current_dir().unwrap().display().to_string();
        assert!(matches!(
            engine.finish().await.as_slice(),
            [Request::ListChats { folder: Some(asked) }] if *asked == folder
        ));
    }

    #[tokio::test]
    async fn resume_all_picks_from_the_chats_of_every_folder() {
        let engine = FakeEngine::start(vec![Reply::result(QueryResult::Chats {
            chats: Vec::new(),
        })]);
        let mut client = engine.client().await;

        let error = resolve_chat(Lang::En, &mut client, OpenMode::PickInAll)
            .await
            .unwrap_err();

        assert!(error.to_string().starts_with("No chat to resume"));
        assert!(matches!(
            engine.finish().await.as_slice(),
            [Request::ListChats { folder: None }]
        ));
    }

    #[test]
    fn add_dir_resolves_to_absolute_folders_and_rejects_files_and_missing_paths() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("lib");
        std::fs::create_dir_all(&folder).unwrap();
        let file = root.path().join("file");
        std::fs::write(&file, "x").unwrap();

        let resolved = resolve_add_dirs(Lang::En, &[folder.join("../lib")]).unwrap();
        let missing = resolve_add_dirs(Lang::En, &[root.path().join("missing")]).unwrap_err();
        let not_folder = resolve_add_dirs(Lang::En, &[file]).unwrap_err();

        assert_eq!(resolved, vec![folder.canonicalize().unwrap()]);
        assert!(missing.to_string().contains("Failed to read --add-dir"));
        assert!(not_folder.to_string().contains("should be a folder"));
        assert!(resolve_add_dirs(Lang::En, &[]).unwrap().is_empty());
    }

    #[test]
    fn mode_for_needs_terminal_input_and_output_for_full_screen() {
        assert_eq!(mode_for(true, true), ScreenMode::FullScreen);
        assert_eq!(mode_for(true, false), ScreenMode::Plain);
        assert_eq!(mode_for(false, true), ScreenMode::Plain);
        assert_eq!(mode_for(false, false), ScreenMode::Plain);
    }

    #[test]
    fn run_options_carries_chat_workdir_and_config_layer_in_order() {
        let config = [entry("permission.mode", "\"full\""), entry("a", "1")];

        let options = run_options(
            Some(ChatId(3)),
            &config,
            vec![PathBuf::from("/shared")],
            PathBuf::from("/work"),
        );

        assert_eq!(options.chat, Some(ChatId(3)));
        assert_eq!(options.workdir, PathBuf::from("/work"));
        assert_eq!(options.add_dirs, vec![PathBuf::from("/shared")]);
        assert_eq!(
            options.overrides,
            vec![
                ("permission.mode".to_owned(), "\"full\"".to_owned()),
                ("a".to_owned(), "1".to_owned())
            ]
        );
        assert!(options.history.ends_with(".saturn/history"));
    }
}
