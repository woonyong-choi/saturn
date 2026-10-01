//! 전체 화면 TUI. engine과는 `protocol` 메시지로만 주고받고 provider를 모른다.
//!
//! 설계: docs/design/tui.md.
//! 흐름: `terminal`이 키·마우스·붙여넣기를 `app::AppEvent`로, `client`가 engine 알림을 `AppEvent::Engine`으로 넘긴다.
//! `app::App`이 상태를 고치고 `app::Effect`(요청 전송, 일시 중지, 종료 등)를 돌려주면 `app::run_loop`가 실행하고 `view`로 다시 그린다.
//! plain 출력(`plain`)은 같은 상태와 같은 문구 함수로 줄만 쓴다.

// TODO(#74): 뼈대 단계라 본문이 `todo!`인 함수의 인자가 쓰이지 않는다. 구현 이슈가 모두 닫히면 이 허용을 지운다
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

/// TUI 실행 오류.
#[derive(Debug, thiserror::Error)]
pub enum TuiError {
    /// engine 연결 오류. 끊기면 화면을 복원하고 끝낸다.
    #[error("engine connection failed")]
    Client(#[from] ClientError),
    /// 터미널 준비, 복원, 일시 중지, 외부 에디터 실패.
    #[error("terminal operation failed")]
    Terminal(#[from] terminal::TerminalError),
    /// 입력 기록 파일 읽기·쓰기 실패.
    #[error("input history failed")]
    History(#[from] history::HistoryError),
    /// plain 출력의 표준 입력 읽기나 표준 출력 쓰기 실패.
    #[error("plain output failed")]
    Plain(#[source] std::io::Error),
}

/// 화면 실행 설정. cli가 채운다.
#[derive(Debug, Clone)]
pub struct RunOptions {
    /// 붙을 채팅. `None`이면 engine이 새 채팅을 만든다(`Request::Attach { chat: None }`).
    pub chat: Option<ChatId>,
    /// 화면 언어. `None`이면 `Lang::detect()`로 운영체제 언어를 따른다.
    pub lang: Option<Lang>,
    /// 작업 폴더. 시작 화면 폴더 칸, `@` 파일 목록, `!` 셸 명령의 실행 위치.
    pub workdir: PathBuf,
    /// 입력 기록 파일. 기본 `~/.saturn/history`(TUI만 쓴다).
    pub history: PathBuf,
}

// cost: time O(e), heap O(s), stack O(1), io e
// vars: e = 처리한 이벤트 수, s = 화면 상태 크기
// basis: estimate
/// 전체 화면 TUI를 연다. 터미널을 대체 화면·raw 모드·붙여넣기 감지·마우스 캡처로 바꾸고 `Attach`를 보낸 뒤
/// 이벤트 루프를 돈다. 끝날 때 `Detach`를 보내고 터미널을 복원한다(오류·panic에서도 복원).
/// TUI를 닫아도 engine은 설정 `on_exit`에 따라 계속 처리한다(기본 `background`).
///
/// # Errors
/// 터미널 설정·복원 실패, engine 연결 끊김, 입력 기록 파일 오류.
pub async fn run(client: &mut EngineClient, options: RunOptions) -> Result<(), TuiError> {
    let lang = options.lang.unwrap_or_else(Lang::detect);
    let history = InputHistory::load(&options.history)?;
    let mut app = App::new(lang, options.workdir, history, options.chat);
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
/// 화면 없는 plain 출력. 표준 입력 한 줄을 입력 하나로 `SubmitInput`하고, 알림을 전체 화면 대화 기록과 같은 문구로
/// 한 줄씩 stdout에 쓴다. 표준 입력이 끝나고 모든 작업이 끝나면(`ChatNotice::RequestSummary`) `Detach`하고 돌아온다.
/// 빈 줄은 보내지 않고, 채팅 id를 알기 전(접속 직후 `HistoryChunk` 전)에 읽은 줄은 모아 두었다가 보낸다.
/// 진단은 stdout이 아니라 `tracing`(stderr)으로 쓴다.
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
    }
}
