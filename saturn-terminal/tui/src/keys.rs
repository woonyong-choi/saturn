//! 키 → 동작 매핑. 순수 함수이고 상태가 필요한 조건은 `KeyContext`로 받는다.
//! 설계: docs/design/tui.md

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use saturn_protocol::rpc::{PermissionAnswer, UsageRange};

use crate::view::popup::PopupKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyArea {
    Transcript,
    StatusBoard,
    Popup,
    Composer,
    RouterKeyPrompt,
    FolderTrust,
    ResumePrompt,
    Permission,
    TaskList,
    FullTranscript,
    Usage,
    RouterVersion,
    TrainConfirm,
    ModelPicker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KeyContext {
    /// 붙여넣은 요소도 없어야 참.
    pub composer_empty: bool,
    pub at_line_start: bool,
    pub at_word_start: bool,
    /// 입력창이 비었거나 불러온 기록 그대로다.
    pub history_browsable: bool,
    pub running: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    // 모든 영역
    ShowFullTranscript,
    Suspend,

    // 공통 목록 조작(팝업, 창, 화면)
    Up,
    Down,
    Confirm,
    Close,
    Quit,

    // 사용량 화면
    UsageRange(UsageRange),

    // 대화 기록(피드백 질문)
    FeedbackDismiss,
    FeedbackAnswer(bool),

    // 상태판(보류 닫기 확인)
    ConfirmCloseHeld,
    KeepHeld,

    // 팝업
    PopupComplete,

    // 입력창
    Insert(char),
    OpenPopup(PopupKind),
    ShowShortcuts,
    Newline,
    Backspace,
    /// TODO(#59): 빈 입력창 `←`로 작업 목록 화면을 열지
    CursorLeft,
    CursorRight,
    RecallLatestInput,
    Interrupt,
    ExternalEditor,
    KillToEnd,
    HistorySearch,
    Yank,
    Submit,
    SubmitQueued,
    ClearSelection,
    HistoryPrev,
    HistoryNext,

    // 폴더 설정 신뢰 창
    TrustApply,

    // 허가 요청 창
    Permission(PermissionAnswer),

    // 작업 목록 화면
    Help,
    NextFilter,
    PrevFilter,
    ToggleFolderScope,
    ContinueHeld,
    CancelOrCloseHeld,
    Search,
    ChangeGroup,
    NewChat,
    RenameChat,
    SendQueued,

    // router 버전 화면
    ResetThresholds,
    TrainFrom,
    UseVersion,
}

/// `global`을 먼저 적용하고, 뜻이 없는 키는 `None`.
pub fn map(area: KeyArea, key: KeyEvent, ctx: KeyContext) -> Option<Action> {
    if let Some(action) = global(key) {
        return Some(action);
    }
    match area {
        KeyArea::Transcript => transcript(key, ctx),
        KeyArea::StatusBoard => status_board(key),
        KeyArea::Popup => popup(key),
        KeyArea::Composer => composer(key, ctx),
        KeyArea::RouterKeyPrompt => router_key_prompt(key),
        KeyArea::FolderTrust => folder_trust(key),
        KeyArea::ResumePrompt => resume_prompt(key),
        KeyArea::Permission => permission(key),
        KeyArea::TaskList => task_list(key),
        KeyArea::FullTranscript => full_transcript(key),
        KeyArea::Usage => usage(key),
        KeyArea::RouterVersion => router_version(key),
        KeyArea::TrainConfirm => train_confirm(key),
        KeyArea::ModelPicker => model_picker(key),
    }
}

pub fn global(key: KeyEvent) -> Option<Action> {
    if is_ctrl(key, 't') {
        Some(Action::ShowFullTranscript)
    } else if is_ctrl(key, 'z') {
        Some(Action::Suspend)
    } else {
        None
    }
}

/// TODO(#54): 입력창이 비었을 때만 받을지, 항상 받을지, `/feedback`으로만 받을지
pub fn transcript(key: KeyEvent, _ctx: KeyContext) -> Option<Action> {
    match key.code {
        KeyCode::Char('0') if is_char(key, '0') => Some(Action::FeedbackDismiss),
        KeyCode::Char('1') if is_char(key, '1') => Some(Action::FeedbackAnswer(true)),
        KeyCode::Char('2') if is_char(key, '2') => Some(Action::FeedbackAnswer(false)),
        _ => None,
    }
}

/// TODO(#52): 상태판 버튼을 `Shift+Tab` 진입과 방향키로 고를지, 명령만 쓸지, 줄마다 번호 키를 줄지
pub fn status_board(key: KeyEvent) -> Option<Action> {
    if is(key, KeyCode::Enter, KeyModifiers::NONE) {
        Some(Action::ConfirmCloseHeld)
    } else if is(key, KeyCode::Esc, KeyModifiers::NONE) {
        Some(Action::KeepHeld)
    } else {
        None
    }
}

/// 그 밖의 키는 `None`으로 입력창에 넘긴다.
pub fn popup(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Enter if key.modifiers.is_empty() => Some(Action::Confirm),
        KeyCode::Esc => Some(Action::Close),
        KeyCode::Tab if key.modifiers.is_empty() => Some(Action::PopupComplete),
        KeyCode::Up if key.modifiers.is_empty() => Some(Action::Up),
        KeyCode::Down if key.modifiers.is_empty() => Some(Action::Down),
        _ => None,
    }
}

pub fn composer(key: KeyEvent, ctx: KeyContext) -> Option<Action> {
    if let Some(action) = composer_control(key, ctx) {
        return Some(action);
    }
    match key.code {
        KeyCode::Char(c) if is_text_input(key) => Some(composer_char(c, ctx)),
        _ => None,
    }
}

pub fn router_key_prompt(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Enter => Some(Action::Confirm),
        KeyCode::Esc => Some(Action::Quit),
        KeyCode::Backspace => Some(Action::Backspace),
        KeyCode::Char(c) if is_text_input(key) => Some(Action::Insert(c)),
        _ => None,
    }
}

/// 설계 표에 `2`가 없어 두 번째 선택지는 방향키와 `Enter`로만 고른다.
pub fn folder_trust(key: KeyEvent) -> Option<Action> {
    if is_ctrl(key, 'c') {
        return Some(Action::Quit);
    }
    match key.code {
        KeyCode::Char('1' | 'y') if is_text_input(key) => Some(Action::TrustApply),
        KeyCode::Char('3' | 'q') if is_text_input(key) => Some(Action::Quit),
        KeyCode::Esc => Some(Action::Quit),
        KeyCode::Enter => Some(Action::Confirm),
        KeyCode::Up => Some(Action::Up),
        KeyCode::Down => Some(Action::Down),
        _ => None,
    }
}

pub fn resume_prompt(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Enter => Some(Action::Confirm),
        KeyCode::Up => Some(Action::Up),
        KeyCode::Down => Some(Action::Down),
        _ => None,
    }
}

/// 1초 입력 보호는 `view::permission`이 건다.
pub fn permission(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Char('y') if is_char(key, 'y') => {
            Some(Action::Permission(PermissionAnswer::AllowOnce))
        }
        KeyCode::Char('a') if is_char(key, 'a') => {
            Some(Action::Permission(PermissionAnswer::AllowAlways))
        }
        KeyCode::Char('d') if is_char(key, 'd') => {
            Some(Action::Permission(PermissionAnswer::Deny { note: None }))
        }
        KeyCode::Esc => Some(Action::Permission(PermissionAnswer::Deny { note: None })),
        _ => None,
    }
}

// cost: time O(1), heap O(1), stack O(1)
// basis: estimate
pub fn task_list(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Enter => Some(Action::Confirm),
        KeyCode::Esc => Some(Action::Close),
        KeyCode::BackTab => Some(Action::PrevFilter),
        KeyCode::Tab if key.modifiers.contains(KeyModifiers::SHIFT) => Some(Action::PrevFilter),
        KeyCode::Tab => Some(Action::NextFilter),
        KeyCode::Up => Some(Action::Up),
        KeyCode::Down => Some(Action::Down),
        KeyCode::Char(c) if is_text_input(key) => task_list_char(c),
        _ => None,
    }
}

/// 닫기와 스크롤 키는 초안.
pub fn full_transcript(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Esc => Some(Action::Close),
        KeyCode::Up => Some(Action::Up),
        KeyCode::Down => Some(Action::Down),
        _ => None,
    }
}

pub fn usage(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Enter => Some(Action::Confirm),
        KeyCode::Esc => Some(Action::Close),
        KeyCode::Char('d') if is_char(key, 'd') => Some(Action::UsageRange(UsageRange::Day)),
        KeyCode::Char('w') if is_char(key, 'w') => Some(Action::UsageRange(UsageRange::Week)),
        _ => None,
    }
}

pub fn router_version(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Enter => Some(Action::Confirm),
        KeyCode::Esc => Some(Action::Close),
        KeyCode::Up => Some(Action::Up),
        KeyCode::Down => Some(Action::Down),
        KeyCode::Char('r') if is_char(key, 'r') => Some(Action::ResetThresholds),
        KeyCode::Char('t') if is_char(key, 't') => Some(Action::TrainFrom),
        KeyCode::Char('u') if is_char(key, 'u') => Some(Action::UseVersion),
        _ => None,
    }
}

pub fn model_picker(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Enter => Some(Action::Confirm),
        KeyCode::Esc => Some(Action::Close),
        KeyCode::Up => Some(Action::Up),
        KeyCode::Down => Some(Action::Down),
        _ => None,
    }
}

pub fn train_confirm(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Enter => Some(Action::Confirm),
        KeyCode::Esc => Some(Action::Close),
        KeyCode::Up => Some(Action::Up),
        KeyCode::Down => Some(Action::Down),
        _ => None,
    }
}

fn composer_control(key: KeyEvent, ctx: KeyContext) -> Option<Action> {
    let ctrl = key.modifiers == KeyModifiers::CONTROL;
    let alt = key.modifiers == KeyModifiers::ALT;
    let shift = key.modifiers == KeyModifiers::SHIFT;
    let plain = key.modifiers.is_empty();
    match key.code {
        KeyCode::Char('c') if ctrl => Some(Action::Interrupt),
        KeyCode::Char('d') if ctrl => ctx.composer_empty.then_some(Action::Quit),
        KeyCode::Char('g') if ctrl => Some(Action::ExternalEditor),
        KeyCode::Char('j') if ctrl => Some(Action::Newline),
        KeyCode::Char('k') if ctrl => Some(Action::KillToEnd),
        KeyCode::Char('r') if ctrl => Some(Action::HistorySearch),
        KeyCode::Char('y') if ctrl => Some(Action::Yank),
        KeyCode::Enter if alt || shift => Some(Action::Newline),
        KeyCode::Enter if plain => Some(Action::Submit),
        KeyCode::Tab if plain && ctx.running => Some(Action::SubmitQueued),
        KeyCode::Tab if plain => Some(Action::Submit),
        KeyCode::Esc => Some(Action::ClearSelection),
        KeyCode::Up if alt => Some(Action::RecallLatestInput),
        KeyCode::Left if shift => Some(Action::RecallLatestInput),
        KeyCode::Up if ctx.history_browsable => Some(Action::HistoryPrev),
        KeyCode::Down if ctx.history_browsable => Some(Action::HistoryNext),
        KeyCode::Up => Some(Action::Up),
        KeyCode::Down => Some(Action::Down),
        KeyCode::Backspace => Some(Action::Backspace),
        KeyCode::Left => Some(Action::CursorLeft),
        KeyCode::Right => Some(Action::CursorRight),
        _ => None,
    }
}

fn composer_char(c: char, ctx: KeyContext) -> Action {
    match c {
        '$' if ctx.at_word_start => Action::OpenPopup(PopupKind::Skill),
        '/' => Action::OpenPopup(PopupKind::Command),
        '@' => Action::OpenPopup(PopupKind::File),
        '?' if ctx.composer_empty => Action::ShowShortcuts,
        _ => Action::Insert(c),
    }
}

fn task_list_char(c: char) -> Option<Action> {
    match c {
        '?' => Some(Action::Help),
        'a' => Some(Action::ToggleFolderScope),
        'c' => Some(Action::ContinueHeld),
        'd' => Some(Action::CancelOrCloseHeld),
        'f' => Some(Action::Search),
        'g' => Some(Action::ChangeGroup),
        'n' => Some(Action::NewChat),
        'r' => Some(Action::RenameChat),
        's' => Some(Action::SendQueued),
        _ => None,
    }
}

/// 수정 키가 없거나 `Shift`뿐이다.
fn is_text_input(key: KeyEvent) -> bool {
    (key.modifiers - KeyModifiers::SHIFT).is_empty()
}

/// 대문자는 crossterm이 보낸 글자 그대로 비교한다.
fn is_char(key: KeyEvent, c: char) -> bool {
    key.code == KeyCode::Char(c) && (key.modifiers - KeyModifiers::SHIFT).is_empty()
}

fn is_ctrl(key: KeyEvent, c: char) -> bool {
    key.code == KeyCode::Char(c) && key.modifiers == KeyModifiers::CONTROL
}

fn is(key: KeyEvent, code: KeyCode, modifiers: KeyModifiers) -> bool {
    key.code == code && key.modifiers == modifiers
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    fn plain(c: char) -> KeyEvent {
        key(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn ctx() -> KeyContext {
        KeyContext {
            composer_empty: true,
            at_line_start: true,
            at_word_start: true,
            history_browsable: true,
            running: false,
        }
    }

    #[test]
    fn map_global_keys_apply_in_every_area() {
        let ctrl_t = key(KeyCode::Char('t'), KeyModifiers::CONTROL);

        assert_eq!(
            map(KeyArea::Permission, ctrl_t, ctx()),
            Some(Action::ShowFullTranscript)
        );
        assert_eq!(
            map(
                KeyArea::TaskList,
                key(KeyCode::Char('z'), KeyModifiers::CONTROL),
                ctx()
            ),
            Some(Action::Suspend)
        );
    }

    #[test]
    fn composer_enter_and_tab_depend_on_running() {
        let tab = key(KeyCode::Tab, KeyModifiers::NONE);
        let running = KeyContext {
            running: true,
            ..ctx()
        };

        assert_eq!(composer(tab, ctx()), Some(Action::Submit));
        assert_eq!(composer(tab, running), Some(Action::SubmitQueued));
        assert_eq!(
            composer(key(KeyCode::Enter, KeyModifiers::NONE), ctx()),
            Some(Action::Submit)
        );
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn composer_newline_keys() {
        for event in [
            key(KeyCode::Enter, KeyModifiers::ALT),
            key(KeyCode::Enter, KeyModifiers::SHIFT),
            key(KeyCode::Char('j'), KeyModifiers::CONTROL),
        ] {
            assert_eq!(composer(event, ctx()), Some(Action::Newline));
        }
    }

    #[test]
    fn composer_special_chars() {
        let typed = KeyContext {
            composer_empty: false,
            at_word_start: false,
            ..ctx()
        };

        assert_eq!(
            composer(plain('/'), ctx()),
            Some(Action::OpenPopup(PopupKind::Command))
        );
        assert_eq!(
            composer(plain('@'), ctx()),
            Some(Action::OpenPopup(PopupKind::File))
        );
        assert_eq!(
            composer(plain('$'), ctx()),
            Some(Action::OpenPopup(PopupKind::Skill))
        );
        assert_eq!(composer(plain('$'), typed), Some(Action::Insert('$')));
        assert_eq!(composer(plain('?'), ctx()), Some(Action::ShowShortcuts));
        assert_eq!(composer(plain('?'), typed), Some(Action::Insert('?')));
        assert_eq!(composer(plain('!'), ctx()), Some(Action::Insert('!')));
    }

    #[test]
    fn composer_ctrl_d_only_on_empty_draft() {
        let ctrl_d = key(KeyCode::Char('d'), KeyModifiers::CONTROL);
        let typed = KeyContext {
            composer_empty: false,
            ..ctx()
        };

        assert_eq!(composer(ctrl_d, ctx()), Some(Action::Quit));
        assert_eq!(composer(ctrl_d, typed), None);
    }

    #[test]
    fn composer_arrows_browse_history_or_move_cursor() {
        let up = key(KeyCode::Up, KeyModifiers::NONE);
        let editing = KeyContext {
            history_browsable: false,
            ..ctx()
        };

        assert_eq!(composer(up, ctx()), Some(Action::HistoryPrev));
        assert_eq!(composer(up, editing), Some(Action::Up));
        assert_eq!(
            composer(key(KeyCode::Up, KeyModifiers::ALT), ctx()),
            Some(Action::RecallLatestInput)
        );
        assert_eq!(
            composer(key(KeyCode::Left, KeyModifiers::SHIFT), ctx()),
            Some(Action::RecallLatestInput)
        );
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn composer_control_keys() {
        let cases = [
            ('c', Action::Interrupt),
            ('g', Action::ExternalEditor),
            ('k', Action::KillToEnd),
            ('r', Action::HistorySearch),
            ('y', Action::Yank),
        ];
        for (c, action) in cases {
            let event = key(KeyCode::Char(c), KeyModifiers::CONTROL);
            assert_eq!(composer(event, ctx()), Some(action));
        }
    }

    #[test]
    fn transcript_accepts_only_feedback_digits() {
        assert_eq!(transcript(plain('0'), ctx()), Some(Action::FeedbackDismiss));
        assert_eq!(
            transcript(plain('1'), ctx()),
            Some(Action::FeedbackAnswer(true))
        );
        assert_eq!(
            transcript(plain('2'), ctx()),
            Some(Action::FeedbackAnswer(false))
        );
        assert_eq!(transcript(plain('3'), ctx()), None);
    }

    #[test]
    fn popup_unknown_keys_fall_through() {
        assert_eq!(
            popup(key(KeyCode::Tab, KeyModifiers::NONE)),
            Some(Action::PopupComplete)
        );
        assert_eq!(popup(plain('a')), None);
    }

    #[test]
    fn folder_trust_keys() {
        assert_eq!(folder_trust(plain('y')), Some(Action::TrustApply));
        assert_eq!(folder_trust(plain('1')), Some(Action::TrustApply));
        assert_eq!(folder_trust(plain('3')), Some(Action::Quit));
        assert_eq!(
            folder_trust(key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Some(Action::Quit)
        );
        assert_eq!(folder_trust(plain('2')), None);
    }

    #[test]
    fn permission_keys() {
        let deny = Some(Action::Permission(PermissionAnswer::Deny { note: None }));
        assert_eq!(
            permission(plain('y')),
            Some(Action::Permission(PermissionAnswer::AllowOnce))
        );
        assert_eq!(
            permission(plain('a')),
            Some(Action::Permission(PermissionAnswer::AllowAlways))
        );
        assert_eq!(permission(plain('d')), deny);
        assert_eq!(permission(key(KeyCode::Esc, KeyModifiers::NONE)), deny);
        assert_eq!(permission(plain('x')), None);
    }

    #[test]
    fn task_list_keys() {
        assert_eq!(
            task_list(key(KeyCode::BackTab, KeyModifiers::SHIFT)),
            Some(Action::PrevFilter)
        );
        assert_eq!(
            task_list(key(KeyCode::Tab, KeyModifiers::NONE)),
            Some(Action::NextFilter)
        );
        assert_eq!(task_list(plain('s')), Some(Action::SendQueued));
        assert_eq!(task_list(plain('a')), Some(Action::ToggleFolderScope));
        assert_eq!(task_list(plain('?')), Some(Action::Help));
    }

    #[test]
    fn router_version_and_status_board_keys() {
        assert_eq!(router_version(plain('u')), Some(Action::UseVersion));
        assert_eq!(router_version(plain('t')), Some(Action::TrainFrom));
        assert_eq!(
            status_board(key(KeyCode::Enter, KeyModifiers::NONE)),
            Some(Action::ConfirmCloseHeld)
        );
        assert_eq!(
            status_board(key(KeyCode::Esc, KeyModifiers::NONE)),
            Some(Action::KeepHeld)
        );
    }

    #[test]
    fn router_key_prompt_takes_text() {
        assert_eq!(router_key_prompt(plain('k')), Some(Action::Insert('k')));
        assert_eq!(
            router_key_prompt(key(KeyCode::Esc, KeyModifiers::NONE)),
            Some(Action::Quit)
        );
    }
}
