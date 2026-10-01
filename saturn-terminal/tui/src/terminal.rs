//! 터미널 준비와 복원, 입력 이벤트 읽기, `Ctrl+Z` 일시 중지, 외부 에디터.
//!
//! crossterm `event-stream` 기능이 없으므로 이벤트는 전용 스레드에서 `crossterm::event::read`로 읽어 tokio 채널로 넘긴다.

use std::io::Stdout;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crossterm::cursor::Show;
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc;

use crate::app::AppEvent;

/// 입력 이벤트 스레드가 이벤트를 기다리는 간격. 일시 중지와 외부 에디터 동안 키를 가로채지 않으려고 짧게 둔다.
const READER_POLL: Duration = Duration::from_millis(50);

/// 일시 중지나 외부 에디터가 터미널을 쓰는 동안 입력 이벤트 스레드를 쉬게 한다.
static READER_PAUSED: AtomicBool = AtomicBool::new(false);

/// 전체 화면 터미널.
pub type Screen = Terminal<CrosstermBackend<Stdout>>;

/// 터미널 조작 오류.
#[derive(Debug, thiserror::Error)]
pub enum TerminalError {
    /// raw 모드, 대체 화면, 붙여넣기 감지, 마우스 캡처를 켜거나 끄지 못했다.
    #[error("failed to configure terminal")]
    Configure(#[source] std::io::Error),
    /// 화면을 그리지 못했다.
    #[error("failed to draw screen")]
    Draw(#[source] std::io::Error),
    /// 일시 중지 신호를 보내지 못했다.
    #[error("failed to suspend")]
    Suspend(#[source] std::io::Error),
    /// 외부 에디터를 띄우지 못했거나 임시 파일을 다루지 못했다.
    #[error("failed to run external editor")]
    Editor(#[source] std::io::Error),
}

/// 전체 화면에 들어간다: raw 모드, 대체 화면, 붙여넣기 감지(1,000자 넘는 붙여넣기 요소용), 마우스 캡처(상태판 버튼 클릭).
/// panic 훅을 걸어 panic 때도 `leave`를 부른다.
///
/// # Errors
/// 터미널 설정 실패면 `Configure`.
pub fn enter() -> Result<Screen, TerminalError> {
    configure().map_err(TerminalError::Configure)?;
    install_panic_hook();
    Terminal::new(CrosstermBackend::new(std::io::stdout())).map_err(TerminalError::Configure)
}

/// 전체 화면에서 나온다. `enter`의 반대 순서로 끄고 커서를 보인다. 여러 번 불러도 된다.
///
/// # Errors
/// 터미널 복원 실패면 `Configure`.
pub fn leave() -> Result<(), TerminalError> {
    let mut stdout = std::io::stdout();
    // 켜지 않은 모드를 끄는 것은 해가 없으므로 여러 번 불러도 된다.
    let restore = execute!(
        stdout,
        DisableMouseCapture,
        DisableBracketedPaste,
        LeaveAlternateScreen,
        Show
    );
    let raw = disable_raw_mode();
    restore.and(raw).map_err(TerminalError::Configure)
}

// cost: time O(1), heap O(1), stack O(1), io 3
// basis: estimate
/// `Ctrl+Z`: `leave` 뒤 자기 프로세스에 `SIGTSTP`(`kill -TSTP <pid>`)를 보내고, `fg`로 돌아오면 `enter`하고 전체를 다시 그린다.
///
/// # Errors
/// 신호 전송 실패면 `Suspend`, 복원 실패면 `Configure`.
pub fn suspend(screen: &mut Screen) -> Result<(), TerminalError> {
    READER_PAUSED.store(true, Ordering::SeqCst);
    leave()?;
    let pid = std::process::id().to_string();
    let signal = std::process::Command::new("kill")
        .args(["-TSTP", &pid])
        .status()
        .map_err(TerminalError::Suspend);
    // `fg`로 돌아오면 여기서 이어진다.
    let restored = configure().map_err(TerminalError::Configure);
    READER_PAUSED.store(false, Ordering::SeqCst);
    signal?;
    restored?;
    screen.clear().map_err(TerminalError::Draw)
}

// cost: time O(n), heap O(n), stack O(1), io 4
// vars: n = 초안과 편집 결과 길이
// basis: estimate
/// `Ctrl+G`: 초안을 임시 파일에 쓰고 외부 에디터를 띄운다. 에디터가 끝나면 파일 내용을 돌려준다.
/// 에디터 순서 `$VISUAL` → `$EDITOR` → `vi`는 초안이다(설계에 없음, docs/design/tui.md 초안 값).
/// 에디터가 도는 동안은 `leave` 상태이고 끝나면 `enter`한다. 끝에 붙은 줄바꿈 하나는 지운다.
///
/// # Errors
/// 에디터 실행이나 임시 파일 처리 실패면 `Editor`.
pub fn edit_external(screen: &mut Screen, draft: &str) -> Result<String, TerminalError> {
    let path = std::env::temp_dir().join(format!("saturn-draft-{}.md", std::process::id()));
    std::fs::write(&path, draft).map_err(TerminalError::Editor)?;
    READER_PAUSED.store(true, Ordering::SeqCst);
    leave()?;
    let editor = editor_program();
    let mut words = editor.split_whitespace();
    let program = words.next().unwrap_or("vi");
    let status = std::process::Command::new(program)
        .args(words)
        .arg(&path)
        .status()
        .map_err(TerminalError::Editor);
    let restored = configure().map_err(TerminalError::Configure);
    READER_PAUSED.store(false, Ordering::SeqCst);
    let content =
        status.and_then(|_| std::fs::read_to_string(&path).map_err(TerminalError::Editor));
    // 지우지 못한 초안 파일은 다음 편집 때 덮어쓴다.
    let _ = std::fs::remove_file(&path);
    restored?;
    screen.clear().map_err(TerminalError::Draw)?;
    let mut text = content?;
    if text.ends_with('\n') {
        text.pop();
    }
    Ok(text)
}

// cost: time O(e), heap O(1), stack O(1), io e
// vars: e = 읽은 터미널 이벤트 수
// basis: estimate
/// 입력 이벤트 스레드를 띄운다. `crossterm::event::read`를 막힌 채 돌며 `AppEvent::Terminal`로 보낸다.
/// 받는 쪽이 닫히면 스레드가 끝난다.
pub fn spawn_event_reader(tx: mpsc::UnboundedSender<AppEvent>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        loop {
            if tx.is_closed() {
                return;
            }
            if READER_PAUSED.load(Ordering::SeqCst) {
                std::thread::sleep(READER_POLL);
                continue;
            }
            match event::poll(READER_POLL) {
                Ok(false) => continue,
                Ok(true) => {}
                Err(error) => {
                    tracing::warn!(%error, "terminal event poll failed");
                    return;
                }
            }
            match event::read() {
                Ok(event) => {
                    if tx.send(AppEvent::Terminal(event)).is_err() {
                        return;
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "terminal event read failed");
                    return;
                }
            }
        }
    })
}

/// raw 모드, 대체 화면, 붙여넣기 감지, 마우스 캡처를 켠다.
fn configure() -> std::io::Result<()> {
    enable_raw_mode()?;
    execute!(
        std::io::stdout(),
        EnterAlternateScreen,
        EnableBracketedPaste,
        EnableMouseCapture
    )
}

/// panic 때도 터미널을 복원한다. 원래 훅은 복원 뒤에 부른다.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // panic 중에는 복원 실패를 알릴 곳이 없다.
        let _ = leave();
        previous(info);
    }));
}

// cost: time O(1), heap O(1), stack O(1), io 2
// basis: estimate
/// 외부 에디터. `$VISUAL` → `$EDITOR` → `vi`에서 처음 비어 있지 않은 것. 공백 뒤는 에디터 인자(`code -w`).
fn editor_program() -> String {
    ["VISUAL", "EDITOR"]
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.is_empty())
        .unwrap_or_else(|| "vi".to_string())
}
