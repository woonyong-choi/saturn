//! 전체 화면 TUI. engine과 `protocol` 메시지로만 주고받고 provider를 모른다.
//! 설계: docs/design/tui.md

pub(crate) mod app;
mod chat_picker;
pub mod client;
pub(crate) mod commands;
pub(crate) mod history;
pub mod i18n;
pub(crate) mod keymap;
pub(crate) mod keys;
pub(crate) mod labels;
pub(crate) mod plain;
pub(crate) mod shell;
pub(crate) mod state;
pub(crate) mod terminal;
pub mod view;

pub(crate) const ROUTER_KEY_ENV: &str = "SATURN_KEY";

use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

use saturn_protocol::ids::ChatId;
use saturn_protocol::rpc::Request;
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::app::App;
use crate::client::{ClientError, EngineClient, Incoming};
use crate::history::InputHistory;
use crate::i18n::Lang;
use crate::plain::PlainOutput;

pub use crate::chat_picker::{chat_summary, pick_chat};

#[derive(Debug, thiserror::Error)]
pub enum TuiError {
    #[error("engine connection failed")]
    Client(#[from] ClientError),
    #[error("terminal operation failed")]
    Terminal(#[from] terminal::TerminalError),
    #[error("input history failed")]
    History(#[from] history::HistoryError),
    #[error("plain output failed")]
    Plain(#[source] std::io::Error),
    #[error("failed to write exit notice")]
    ExitNotice(#[source] std::io::Error),
    /// 화면이 없어 키를 묻지 않고 방법을 안내하고 끝낸다.
    #[error(
        "router key required ({reason}): set the SATURN_KEY environment variable or the router.key.command setting"
    )]
    RouterKeyRequired { reason: String },
    /// 사용자가 router 키나 폴더 신뢰 창을 닫아 끝냈다.
    #[error("quit from a prompt window")]
    Aborted,
    /// plain 모드에서 provider 작업이 실패로 끝났다. 실패 줄은 이미 출력했다.
    #[error("a task failed")]
    TaskFailed,
}

#[derive(Debug, Clone)]
pub struct RunOptions {
    /// `None`이면 engine이 새 채팅을 만든다.
    pub chat: Option<ChatId>,
    /// `None`이면 운영체제 언어를 따른다.
    pub lang: Option<Lang>,
    pub workdir: PathBuf,
    /// `-c key=value` 실행 층. `Attach`의 `overrides`로 넘긴다.
    pub overrides: Vec<(String, String)>,
    /// `--add-dir`로 받은 폴더의 절대 경로. `Attach`의 `add_dirs`로 넘긴다.
    pub add_dirs: Vec<PathBuf>,
    /// 기본 `~/.saturn/history`.
    pub history: PathBuf,
    /// 에이전트 작업 안에서 출입증으로 붙을 때만 둔다. 있으면 `Attach` 대신 `AttachChild`를 보낸다.
    pub child: Option<ChildAccess>,
}

/// 하위 접속의 출입증과 요청 모드. 작업 폴더, 환경, 더한 폴더, 실행 층은 engine이 부모에게서 물려주므로 없다.
#[derive(Clone)]
pub struct ChildAccess {
    pub pass: String,
    /// 부모 모드를 넘지 않는 모드. `None`이면 부모 모드.
    pub mode: Option<String>,
}

impl std::fmt::Debug for ChildAccess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChildAccess")
            .field("mode", &self.mode)
            .finish_non_exhaustive()
    }
}

// cost: time O(d), heap O(d), stack O(1), alloc d
// vars: d = 폴더 수
// basis: estimate
fn path_texts(paths: &[PathBuf]) -> Vec<String> {
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect()
}

// cost: time O(e), heap O(s), stack O(1), io e
// vars: e = 처리한 이벤트 수, s = 화면 상태 크기
// basis: estimate
/// 오류나 panic으로 끝나도 터미널을 복원한다.
///
/// # Errors
/// 터미널 설정·복원 실패, engine 연결 끊김, 입력 기록 파일 오류.
pub async fn run(client: &mut EngineClient, options: RunOptions) -> Result<(), TuiError> {
    let lang = options.lang.unwrap_or_else(Lang::detect);
    let history = InputHistory::load(&options.history)?;
    let mut app = App::new(lang, options.workdir, history, options.chat);
    app.env = client::attach_env();
    app.overrides = options.overrides;
    app.add_dirs = path_texts(&options.add_dirs);
    let mut screen = terminal::enter()?;
    let result = app::run_loop(&mut app, client, &mut screen).await;
    let restored = terminal::leave();
    result?;
    restored?;
    if app.aborted {
        return Err(TuiError::Aborted);
    }
    if let Some(line) = app.exit_line() {
        writeln!(std::io::stdout().lock(), "{line}").map_err(TuiError::ExitNotice)?;
    }
    Ok(())
}

// cost: time O(e), heap O(w), stack O(1), io e
// vars: e = 표준 입력 줄 수 + 받은 알림 수, w = 보내기를 기다리는 줄 글자 수
// basis: estimate
/// stdout에는 대화 기록 줄만 쓰고 진단은 `tracing`(stderr)으로 보낸다.
/// TODO(#57): plain을 켜는 조건과 우선순위, 설정 키 이름
///
/// # Errors
/// 표준 입출력 실패, engine 연결 끊김.
pub async fn run_plain(client: &mut EngineClient, options: RunOptions) -> Result<(), TuiError> {
    let lang = options.lang.unwrap_or_else(Lang::detect);
    let mut output = PlainOutput::new(std::io::stdout(), lang);
    if let Some(child) = options.child {
        // 상한이 차면 자리가 날 때까지 응답이 없다. 거절은 오류로 끝낸다
        let mut written = Ok(());
        let request = Request::AttachChild {
            pass: child.pass,
            mode: child.mode,
        };
        client
            .call(request, |notification| {
                if written.is_ok() {
                    written = output.apply(notification, Instant::now());
                }
            })
            .await?;
        written.map_err(TuiError::Plain)?;
    } else {
        client
            .send(Request::Attach {
                chat: options.chat,
                workdir: options.workdir.display().to_string(),
                env: client::attach_env(),
                overrides: options.overrides,
                add_dirs: path_texts(&options.add_dirs),
            })
            .await?;
    }
    let mut stdin = BufReader::new(tokio::io::stdin()).lines();
    let mut waiting: VecDeque<String> = VecDeque::new();
    let mut stdin_open = true;
    let mut submitted = 0_u64;
    loop {
        if let Some(chat) = output.chat() {
            while let Some(text) = waiting.pop_front() {
                submitted += 1;
                output.submitted(text.clone());
                let request = Request::SubmitInput {
                    chat,
                    client_ref: submitted,
                    text,
                    skip_relation: false,
                };
                client.send(request).await?;
            }
        }
        let idle = submitted == 0 || output.is_finished();
        if !stdin_open && waiting.is_empty() && idle {
            client.send(Request::Detach).await?;
            return if output.has_failed() {
                Err(TuiError::TaskFailed)
            } else {
                Ok(())
            };
        }
        tokio::select! {
            line = stdin.next_line(), if stdin_open => match line.map_err(TuiError::Plain)? {
                Some(line) if !line.trim().is_empty() => waiting.push_back(line),
                Some(_) => {}
                None => stdin_open = false,
            },
            incoming = client.next() => match incoming {
                Some(Incoming::Notification(notification)) => output
                    .apply(notification, Instant::now())
                    .map_err(TuiError::Plain)?,
                Some(Incoming::Result(_)) => {}
                None => return Err(ClientError::Closed.into()),
            },
        }
        if let Some(reason) = output.key_required() {
            return Err(TuiError::RouterKeyRequired {
                reason: reason.to_owned(),
            });
        }
    }
}
