//! 터미널 준비와 복원, 입력 이벤트 읽기, `Ctrl+Z` 일시 중지, 외부 에디터.
//!
//! crossterm `event-stream` 기능이 없으므로 이벤트는 전용 스레드에서 `crossterm::event::read`로 읽어 tokio 채널로 넘긴다.

use std::io::Stdout;

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc;

use crate::app::AppEvent;

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
    todo!("#92")
}

/// 전체 화면에서 나온다. `enter`의 반대 순서로 끄고 커서를 보인다. 여러 번 불러도 된다.
///
/// # Errors
/// 터미널 복원 실패면 `Configure`.
pub fn leave() -> Result<(), TerminalError> {
    todo!("#92")
}

/// `Ctrl+Z`: `leave` 뒤 자기 프로세스에 `SIGTSTP`(`kill -TSTP <pid>`)를 보내고, `fg`로 돌아오면 `enter`하고 전체를 다시 그린다.
///
/// # Errors
/// 신호 전송 실패면 `Suspend`, 복원 실패면 `Configure`.
pub fn suspend(screen: &mut Screen) -> Result<(), TerminalError> {
    todo!("#92")
}

/// `Ctrl+G`: 초안을 임시 파일에 쓰고 외부 에디터를 띄운다. 에디터가 끝나면 파일 내용을 돌려준다. 에디터 순서는 초안이다(설계에 없음).
/// TODO(#92): 값 미정, 초안 `$VISUAL` → `$EDITOR` → `vi`
/// 에디터가 도는 동안은 `leave` 상태이고 끝나면 `enter`한다. 끝에 붙은 줄바꿈 하나는 지운다.
///
/// # Errors
/// 에디터 실행이나 임시 파일 처리 실패면 `Editor`.
pub fn edit_external(screen: &mut Screen, draft: &str) -> Result<String, TerminalError> {
    todo!("#92")
}

/// 입력 이벤트 스레드를 띄운다. `crossterm::event::read`를 막힌 채 돌며 `AppEvent::Terminal`로 보낸다.
/// 받는 쪽이 닫히면 스레드가 끝난다.
pub fn spawn_event_reader(tx: mpsc::UnboundedSender<AppEvent>) -> std::thread::JoinHandle<()> {
    todo!("#92")
}
