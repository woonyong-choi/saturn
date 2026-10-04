//! 프리셋 등록과 표. 프리셋은 기본 표에서 다른 줄만 적고, 적지 않은 키는 기본 표를 물려받는다.
//! 키가 없는 줄은 그 동작의 키를 걷는다(명령은 남는다).
//! 근거는 각 도구의 공식 키 문서이고 표 위에 적었다. 설계: docs/design/tui.md

use super::{Binding, Cond};
use crate::keys::Action;
use crate::keys::KeyArea as A;

pub(crate) struct Preset {
    pub name: &'static str,
    pub overrides: fn() -> Vec<Binding>,
}

/// 새 프리셋은 표 함수 하나와 이 목록의 한 줄이다. 이름은 `saturn_protocol::keymap::PRESET_NAMES`와 같아야 한다.
pub(crate) const PRESETS: &[Preset] = &[
    Preset {
        name: "saturn",
        overrides: saturn,
    },
    Preset {
        name: "claude",
        overrides: claude,
    },
    Preset {
        name: "codex",
        overrides: codex,
    },
    Preset {
        name: "gemini",
        overrides: gemini,
    },
    Preset {
        name: "opencode",
        overrides: opencode,
    },
];

fn b(action: Action, scopes: &[A], keys: &[&'static str]) -> Binding {
    Binding::new(action, scopes, keys)
}

/// 기본 표 그대로다.
fn saturn() -> Vec<Binding> {
    Vec::new()
}

/// 근거: Claude Code `interactive-mode` 키 표. 기록 보기가 `Ctrl+O`(`Ctrl+T`는 할 일 목록), 대기 전송이 `Ctrl+X Enter`다.
/// `Esc`, `Esc Esc`, `Shift+Tab`, `Tab`, `Ctrl+C`는 기본 표와 같다.
fn claude() -> Vec<Binding> {
    vec![
        b(Action::ShowFullTranscript, &[A::Global], &["ctrl+o"]),
        b(Action::SubmitQueued, &[A::Composer], &["ctrl+x enter"]),
    ]
}

/// 공식 키 표가 없다. 문서에서 확인한 `Esc Esc`(앞 메시지 고치기)와 `Ctrl+T`(기록 보기)만 적었고, 확인하지 못한 키는
/// 기본 표 값을 유지한다. 그래서 기본 표와 같다.
fn codex() -> Vec<Binding> {
    Vec::new()
}

/// 근거: Gemini CLI `keyboard-shortcuts` 문서. `Esc`는 창과 포커스만 닫고(작업 중단 아님), `Esc Esc`는 입력 지우기나 이전
/// 입력 보기, `Tab`은 제안 수락이거나 작업 중이면 메시지를 뒤로 대기, `Ctrl+C`는 입력이 비면 바로 종료, `Ctrl+O`는 내용
/// 펼치기, `Ctrl+Y`는 모두 허용 전환이라 되돌려 붙이기를 `Alt+Y`로 옮긴다.
fn gemini() -> Vec<Binding> {
    vec![
        b(Action::StopWork, &[A::Composer], &[]),
        b(Action::ClearSelection, &[A::Composer], &["esc"]),
        b(Action::SubmitQueued, &[A::Composer], &["tab"]).when(Cond::Running),
        b(Action::Interrupt, &[A::Fallback], &[]),
        b(Action::InterruptQuit, &[A::Fallback], &["ctrl+c"]).command("/stop"),
        b(Action::ShowFullTranscript, &[A::Global], &["ctrl+o"]),
        b(Action::Yank, &[A::Composer], &["alt+y"]),
    ]
}

/// 근거: OpenCode `keybinds` 문서. `Tab`이 에이전트 순환이라 권한 모드 순환에 쓰고 명령 목록 키 `Ctrl+P`(`command_list`)를
/// 명령 목록 열기에 쓴다(팝업 안의 `Tab`은 그대로). `Esc Esc`는 없다. `Ctrl+C`는 입력 지우기와 종료로 기본 표와 같다.
fn opencode() -> Vec<Binding> {
    vec![
        b(
            Action::CyclePermissionMode,
            &[A::Composer],
            &["tab", "shift+tab"],
        ),
        b(Action::CompleteCommand, &[A::Composer], &["ctrl+p"]),
        b(Action::Rewind, &[A::Composer], &[]),
    ]
}
