//! 전체 화면 TUI. engine과 `protocol` 메시지로만 주고받고 provider를 모른다.
//! 설계: docs/design/tui.md

// TODO(#74): 뼈대의 `todo!` 본문 때문에 둔 허용, 구현이 끝나면 지운다
#![allow(unused_variables, dead_code)]

pub mod app;
pub mod client;
pub mod commands;
pub mod history;
pub mod i18n;
pub mod keys;
pub mod labels;
pub mod plain;
pub mod shell;
pub mod state;
pub mod terminal;
pub mod view;

pub(crate) const JUDGE_KEY_ENV: &str = "SATURN_KEY";

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Instant;

use saturn_protocol::ids::ChatId;
use saturn_protocol::rpc::Request;
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::app::App;
use crate::client::{ClientError, EngineClient};
use crate::history::InputHistory;
use crate::i18n::Lang;
use crate::plain::PlainOutput;

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
    /// 화면이 없어 키를 묻지 않고 방법을 안내하고 끝낸다.
    #[error(
        "judge key required ({reason}): set the SATURN_KEY environment variable or the judge.key.command setting"
    )]
    JudgeKeyRequired { reason: String },
}

#[derive(Debug, Clone)]
pub struct RunOptions {
    /// `None`이면 engine이 새 채팅을 만든다.
    pub chat: Option<ChatId>,
    /// `None`이면 운영체제 언어를 따른다.
    pub lang: Option<Lang>,
    pub workdir: PathBuf,
    /// 기본 `~/.saturn/history`.
    pub history: PathBuf,
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
    let mut screen = terminal::enter()?;
    let result = app::run_loop(&mut app, client, &mut screen).await;
    let restored = terminal::leave();
    result?;
    restored?;
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
    client
        .send(Request::Attach {
            chat: options.chat,
            workdir: options.workdir.display().to_string(),
            env: client::attach_env(),
            overrides: Vec::new(),
        })
        .await?;
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
                    pinned_model: None,
                    skip_relation: false,
                };
                client.send(request).await?;
            }
        }
        let idle = submitted == 0 || output.is_finished();
        if !stdin_open && waiting.is_empty() && idle {
            client.send(Request::Detach).await?;
            return Ok(());
        }
        tokio::select! {
            line = stdin.next_line(), if stdin_open => match line.map_err(TuiError::Plain)? {
                Some(line) if !line.trim().is_empty() => waiting.push_back(line),
                Some(_) => {}
                None => stdin_open = false,
            },
            notification = client.next() => match notification {
                Some(notification) => output
                    .apply(notification, Instant::now())
                    .map_err(TuiError::Plain)?,
                None => return Err(ClientError::Closed.into()),
            },
        }
        if let Some(reason) = output.key_required() {
            return Err(TuiError::JudgeKeyRequired {
                reason: reason.to_owned(),
            });
        }
    }
}
