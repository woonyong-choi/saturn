//! 키 표 → 동작. 영역마다 매핑 함수 하나, 모든 영역에 먼저 `global`을 적용한다.
//!
//! 설계: docs/design/tui.md(키). 어느 영역이 키를 받는지는 `app::App::key_area`가 정한다.
//! 매핑은 순수 함수다. 상태가 필요한 조건(빈 입력창, 줄 앞, 실행 중)은 `KeyContext`로 받는다.
//! `Action`의 실행(요청 전송, 창 열기)은 `app::App::on_action`이 한다.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use saturn_protocol::rpc::PermissionAnswer;

use crate::view::popup::PopupKind;

/// 키를 받는 영역. 설계 표의 적용 영역 열과 같다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyArea {
    /// 대화 기록. 피드백 질문이 떠 있을 때만 숫자 키를 받는다.
    Transcript,
    /// 상태판. 보류 닫기 확인이 떠 있을 때만.
    StatusBoard,
    /// `/`, `@`, `$` 팝업.
    Popup,
    /// 입력창.
    Composer,
    /// judge 키 입력 창.
    JudgeKeyPrompt,
    /// 폴더 설정 신뢰 창.
    FolderTrust,
    /// 보류 재개 질문.
    ResumePrompt,
    /// 허가 요청 창.
    Permission,
    /// 작업 목록 화면(`/tasks`).
    TaskList,
    /// 전체 기록(`Ctrl+T`).
    FullTranscript,
    /// 사용량 화면(`/usage`).
    Usage,
    /// judge 버전 화면(`/judge version`).
    JudgeVersion,
    /// 학습 확인 창(`/train`).
    TrainConfirm,
}

/// 매핑에 필요한 화면 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KeyContext {
    /// 입력창이 비었다(붙여넣은 요소도 없음).
    pub composer_empty: bool,
    /// 커서가 줄 맨 앞이다(`!` 셸 명령).
    pub at_line_start: bool,
    /// 커서가 단어 맨 앞이다(`$` 스킬 목록).
    pub at_word_start: bool,
    /// 입력창이 비었거나 불러온 기록 그대로다(`↑`, `↓` 기록 이동).
    pub history_browsable: bool,
    /// 살아 있는 작업이 하나라도 있다(`Enter`, `Tab`, `Ctrl+C` 뜻이 바뀐다).
    pub running: bool,
}

/// 키가 뜻하는 동작. 설계 표 한 줄이 variant 하나다(같은 동작을 여러 영역이 쓰면 공유).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    // 모든 영역
    /// `Ctrl+T` 전체 기록 표시. 열려 있으면 닫는다.
    ShowFullTranscript,
    /// `Ctrl+Z` 화면 일시 중지, `fg` 뒤 복원.
    Suspend,

    // 공통 목록 조작(팝업, 창, 화면)
    /// `↑` 목록·선택지 이동.
    Up,
    /// `↓` 목록·선택지 이동.
    Down,
    /// `Enter` 강조한 항목 확정. 영역마다 뜻이 다르다(선택, 상세, 이동).
    Confirm,
    /// `Esc` 닫기·취소. 영역마다 뜻이 다르다(팝업 해제, 화면 종료, 보류 유지).
    Close,
    /// 종료. judge 키 입력 창 `Esc`, 폴더 신뢰 창 `3`/`q`/`Esc`/`Ctrl+C`, 빈 입력창 `Ctrl+D`, 유휴 `Ctrl+C`.
    Quit,

    // 대화 기록(피드백 질문)
    /// `0` 피드백 질문 해제.
    FeedbackDismiss,
    /// `1` 맞아요(`true`), `2` 아니에요(`false`). `/feedback 1`, `/feedback 2`와 같다.
    FeedbackAnswer(bool),

    // 상태판(보류 닫기 확인)
    /// `Enter` 보류 종료(`Request::CloseHeld`).
    ConfirmCloseHeld,
    /// `Esc` 보류 유지.
    KeepHeld,

    // 팝업
    /// `Tab` 명령의 전체 경로까지 완성.
    PopupComplete,

    // 입력창
    /// 글자 하나 넣기. `!`, `$`, `/`, `@`, `?`의 특별한 뜻은 `OpenPopup`, `ShowShortcuts`로 따로 온다.
    Insert(char),
    /// `/` 명령 목록, `@` 파일 목록, `$` 스킬 목록(단어 맨 앞). 글자도 함께 넣는다.
    OpenPopup(PopupKind),
    /// `?` 빈 입력창에서 단축키 안내.
    ShowShortcuts,
    /// `Alt+Enter`, `Ctrl+J`, `Shift+Enter` 줄바꿈.
    Newline,
    /// `Backspace` 앞 글자 지우기. 붙여넣은 요소 바로 뒤면 요소 전체.
    Backspace,
    /// `←` 커서 이동. TODO(#59): 빈 입력창 `←`로 작업 목록 화면을 열지
    CursorLeft,
    /// `→` 커서 이동.
    CursorRight,
    /// `Alt+↑`, `Shift+←` 대기나 판단 중인 가장 최근 입력을 취소하고 원문을 입력창으로.
    RecallLatestInput,
    /// `Ctrl+C` 뷰 해제 → 검색 취소 → 초안 삭제 순서로 하나, 그 밖에는 실행 중이면 멈춤과 보류, 유휴이면 종료.
    Interrupt,
    /// `Ctrl+G` 외부 에디터로 초안 편집.
    ExternalEditor,
    /// `Ctrl+K` 커서부터 줄 끝까지 잘라 보관.
    KillToEnd,
    /// `Ctrl+R` 입력 기록 검색.
    HistorySearch,
    /// `Ctrl+Y` 잘라 둔 글자 복원.
    Yank,
    /// `Enter` 입력 제출. 유휴이면 새 작업, 실행 중이면 judge가 끼워 넣기·새 작업·대기 중 하나. `!` 줄이면 셸 명령 실행.
    Submit,
    /// `Tab` 유휴이면 `Submit`과 같고, 실행 중이면 관계 판단 없이 대기(`skip_relation: true`, 보낼 때 judge 1회).
    SubmitQueued,
    /// `Esc` 팝업과 선택 해제. 작업을 멈추지 않는다.
    ClearSelection,
    /// `↑` 입력 기록 이전.
    HistoryPrev,
    /// `↓` 입력 기록 다음.
    HistoryNext,

    // 폴더 설정 신뢰 창
    /// `1`, `y` 적용하고 계속 강조.
    TrustApply,

    // 허가 요청 창
    /// `y` 실행, `a` 이 작업 동안 같은 명령 허용, `d` 실행하지 않고 계속, `Esc` 실행하지 않고 다르게 하라고 말하기.
    Permission(PermissionAnswer),

    // 작업 목록 화면
    /// `?` 도움말.
    Help,
    /// `Tab` 다음 필터.
    NextFilter,
    /// `Shift+Tab` 이전 필터.
    PrevFilter,
    /// `c` 보류 작업 재개.
    ContinueHeld,
    /// `d` 대기 취소, 보류면 확인 한 줄 뒤 보류 종료.
    CancelOrCloseHeld,
    /// `f` 검색.
    Search,
    /// `g` 묶음 변경.
    ChangeGroup,
    /// `n` 새 채팅.
    NewChat,
    /// `r` 채팅 이름 변경.
    RenameChat,
    /// `s` 대기 입력 전송.
    SendQueued,

    // judge 버전 화면
    /// `r` 1차 영점으로 복귀. `/train --reset-thresholds`와 같다.
    ResetThresholds,
    /// `t` 고른 버전에서 다시 학습. `/train --from`과 같다.
    TrainFrom,
    /// `u` 확인 한 줄 뒤 고른 버전 사용. `saturn judge version`과 같다.
    UseVersion,
}

/// 영역의 키를 동작으로 바꾼다. `global`을 먼저, 없으면 영역 함수. 뜻이 없는 키는 `None`(무시).
pub fn map(area: KeyArea, key: KeyEvent, ctx: KeyContext) -> Option<Action> {
    if let Some(action) = global(key) {
        return Some(action);
    }
    match area {
        KeyArea::Transcript => transcript(key, ctx),
        KeyArea::StatusBoard => status_board(key),
        KeyArea::Popup => popup(key),
        KeyArea::Composer => composer(key, ctx),
        KeyArea::JudgeKeyPrompt => judge_key_prompt(key),
        KeyArea::FolderTrust => folder_trust(key),
        KeyArea::ResumePrompt => resume_prompt(key),
        KeyArea::Permission => permission(key),
        KeyArea::TaskList => task_list(key),
        KeyArea::FullTranscript => full_transcript(key),
        KeyArea::Usage => usage(key),
        KeyArea::JudgeVersion => judge_version(key),
        KeyArea::TrainConfirm => train_confirm(key),
    }
}

/// 모든 영역: `Ctrl+T` → `ShowFullTranscript`, `Ctrl+Z` → `Suspend`.
pub fn global(key: KeyEvent) -> Option<Action> {
    if is_ctrl(key, 't') {
        Some(Action::ShowFullTranscript)
    } else if is_ctrl(key, 'z') {
        Some(Action::Suspend)
    } else {
        None
    }
}

/// 대화 기록: `0` → `FeedbackDismiss`, `1` → `FeedbackAnswer(true)`, `2` → `FeedbackAnswer(false)`.
/// 피드백 질문이 떠 있을 때만 이 영역이 된다. 결정 전이라 가장 단순하게 입력창 상태와 관계없이 늘 받는다(`_ctx`는 결정 뒤 쓴다).
/// TODO(#54): 입력창이 비었을 때만 받을지, 항상 받을지, `/feedback`으로만 받을지
pub fn transcript(key: KeyEvent, _ctx: KeyContext) -> Option<Action> {
    match key.code {
        KeyCode::Char('0') if is_char(key, '0') => Some(Action::FeedbackDismiss),
        KeyCode::Char('1') if is_char(key, '1') => Some(Action::FeedbackAnswer(true)),
        KeyCode::Char('2') if is_char(key, '2') => Some(Action::FeedbackAnswer(false)),
        _ => None,
    }
}

/// 상태판(보류 닫기 확인): `Enter` → `ConfirmCloseHeld`, `Esc` → `KeepHeld`.
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

/// 팝업: `Enter` → `Confirm`(명령 목록이면 전체 경로를 입력창에 기입, 값 목록이면 값 선택),
/// `Esc` → `Close`(입력 토큰이 바뀔 때까지 재표시 없음), `Tab` → `PopupComplete`, `↑`/`↓` → `Up`/`Down`.
/// 그 밖의 키는 `None`을 돌려 입력창으로 넘긴다(글자 입력마다 목록 필터).
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

/// 입력창.
/// - `!`: 줄 앞이면 `Insert('!')`(셸 명령 줄이 된다).
/// - `$`: 단어 맨 앞이면 `OpenPopup(Skill)`. 다른 provider 스킬은 `$공급자 이름`.
/// - `/` → `OpenPopup(Command)`, `@` → `OpenPopup(File)`, `?`: 빈 입력창이면 `ShowShortcuts`, 아니면 `Insert('?')`.
/// - `Alt+Enter`, `Ctrl+J`, `Shift+Enter` → `Newline`. `Alt+↑`, `Shift+←` → `RecallLatestInput`.
/// - `Ctrl+C` → `Interrupt`, `Ctrl+D`: 빈 입력창이면 `Quit`, `Ctrl+G` → `ExternalEditor`, `Ctrl+K` → `KillToEnd`,
///   `Ctrl+R` → `HistorySearch`, `Ctrl+Y` → `Yank`.
/// - `Enter` → `Submit`, `Tab` → `SubmitQueued`(유휴면 `Submit`), `Esc` → `ClearSelection`.
/// - `↑`/`↓`: `history_browsable`이면 `HistoryPrev`/`HistoryNext`, 아니면 줄 사이 커서 이동(`Up`/`Down`).
/// - `Backspace`, `←`, `→`, 그 밖의 글자는 편집 동작.
pub fn composer(key: KeyEvent, ctx: KeyContext) -> Option<Action> {
    if let Some(action) = composer_control(key, ctx) {
        return Some(action);
    }
    match key.code {
        KeyCode::Char(c) if is_text_input(key) => Some(composer_char(c, ctx)),
        _ => None,
    }
}

/// judge 키 입력 창: `Enter` → `Confirm`(키 확인), `Esc` → `Quit`. 그 밖의 글자는 가린 입력칸에 `Insert`.
pub fn judge_key_prompt(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Enter => Some(Action::Confirm),
        KeyCode::Esc => Some(Action::Quit),
        KeyCode::Backspace => Some(Action::Backspace),
        KeyCode::Char(c) if is_text_input(key) => Some(Action::Insert(c)),
        _ => None,
    }
}

/// 폴더 설정 신뢰 창: `1`, `y` → `TrustApply`, `3`, `q`, `Esc`, `Ctrl+C` → `Quit`, `Enter` → `Confirm`, `↑`/`↓` → `Up`/`Down`.
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

/// 보류 재개 질문: `Enter` → `Confirm`, `↑`/`↓` → `Up`/`Down`.
pub fn resume_prompt(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Enter => Some(Action::Confirm),
        KeyCode::Up => Some(Action::Up),
        KeyCode::Down => Some(Action::Down),
        _ => None,
    }
}

/// 허가 요청 창: `y` → `Allow`, `a` → `AllowForTask`, `d` → `Deny`, `Esc` → `DenyAndRedirect`.
/// 창이 뜬 뒤 1초 입력 보호는 `view::permission`이 건다(여기서는 매핑만).
pub fn permission(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Char('y') if is_char(key, 'y') => {
            Some(Action::Permission(PermissionAnswer::Allow))
        }
        KeyCode::Char('a') if is_char(key, 'a') => {
            Some(Action::Permission(PermissionAnswer::AllowForTask))
        }
        KeyCode::Char('d') if is_char(key, 'd') => Some(Action::Permission(PermissionAnswer::Deny)),
        KeyCode::Esc => Some(Action::Permission(PermissionAnswer::DenyAndRedirect)),
        _ => None,
    }
}

// cost: time O(1), heap O(1), stack O(1)
// basis: estimate
/// 작업 목록 화면: `?` → `Help`, `Enter` → `Confirm`(그 채팅으로 이동해 해당 작업 결과로 스크롤), `Esc` → `Close`,
/// `Tab`/`Shift+Tab` → `NextFilter`/`PrevFilter`, `c` → `ContinueHeld`, `d` → `CancelOrCloseHeld`, `f` → `Search`,
/// `g` → `ChangeGroup`, `n` → `NewChat`, `r` → `RenameChat`, `s` → `SendQueued`, `↑`/`↓` → `Up`/`Down`.
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

/// 전체 기록: `Esc` → `Close`, `↑`/`↓` → `Up`/`Down`(스크롤). 설계 표에는 `Ctrl+T` 표시만 있어 닫기와 스크롤 키는 초안이다
/// (docs/design/tui.md 초안 값). `Ctrl+T`를 다시 누르면 `global`이 닫는다.
pub fn full_transcript(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Esc => Some(Action::Close),
        KeyCode::Up => Some(Action::Up),
        KeyCode::Down => Some(Action::Down),
        _ => None,
    }
}

/// 사용량 화면: `Enter` → `Confirm`(judge 실제 모델 상세), `Esc` → `Close`.
pub fn usage(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Enter => Some(Action::Confirm),
        KeyCode::Esc => Some(Action::Close),
        _ => None,
    }
}

/// judge 버전 화면: `Enter` → `Confirm`(버전 상세), `Esc` → `Close`, `r` → `ResetThresholds`, `t` → `TrainFrom`,
/// `u` → `UseVersion`, `↑`/`↓` → `Up`/`Down`.
pub fn judge_version(key: KeyEvent) -> Option<Action> {
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

/// 학습 확인 창: `Enter` → `Confirm`, `Esc` → `Close`(취소), `↑`/`↓` → `Up`/`Down`.
pub fn train_confirm(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Enter => Some(Action::Confirm),
        KeyCode::Esc => Some(Action::Close),
        KeyCode::Up => Some(Action::Up),
        KeyCode::Down => Some(Action::Down),
        _ => None,
    }
}

/// 입력창의 `Ctrl`/`Alt` 조합과 특수 키. 글자 입력이 아니면 `None`.
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

/// 입력창 글자 하나. `!`, `$`, `/`, `@`, `?`의 특별한 뜻을 가른다.
fn composer_char(c: char, ctx: KeyContext) -> Action {
    match c {
        '$' if ctx.at_word_start => Action::OpenPopup(PopupKind::Skill),
        '/' => Action::OpenPopup(PopupKind::Command),
        '@' => Action::OpenPopup(PopupKind::File),
        '?' if ctx.composer_empty => Action::ShowShortcuts,
        _ => Action::Insert(c),
    }
}

/// 작업 목록 화면의 글자 키.
fn task_list_char(c: char) -> Option<Action> {
    match c {
        '?' => Some(Action::Help),
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

/// 글자 입력인지. 수정 키가 없거나 `Shift`뿐이다(`Ctrl`, `Alt` 조합은 명령).
fn is_text_input(key: KeyEvent) -> bool {
    (key.modifiers - KeyModifiers::SHIFT).is_empty()
}

/// 수정 키 없이 이 글자인지. 대문자와 `Shift`는 crossterm이 글자로 보낸 값 그대로 비교한다.
fn is_char(key: KeyEvent, c: char) -> bool {
    key.code == KeyCode::Char(c) && (key.modifiers - KeyModifiers::SHIFT).is_empty()
}

/// `Ctrl`과 이 글자인지.
fn is_ctrl(key: KeyEvent, c: char) -> bool {
    key.code == KeyCode::Char(c) && key.modifiers == KeyModifiers::CONTROL
}

/// 수정 키와 코드가 정확히 같은지.
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
        assert_eq!(
            permission(plain('a')),
            Some(Action::Permission(PermissionAnswer::AllowForTask))
        );
        assert_eq!(
            permission(key(KeyCode::Esc, KeyModifiers::NONE)),
            Some(Action::Permission(PermissionAnswer::DenyAndRedirect))
        );
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
        assert_eq!(task_list(plain('?')), Some(Action::Help));
    }

    #[test]
    fn judge_version_and_status_board_keys() {
        assert_eq!(judge_version(plain('u')), Some(Action::UseVersion));
        assert_eq!(judge_version(plain('t')), Some(Action::TrainFrom));
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
    fn judge_key_prompt_takes_text() {
        assert_eq!(judge_key_prompt(plain('k')), Some(Action::Insert('k')));
        assert_eq!(
            judge_key_prompt(key(KeyCode::Esc, KeyModifiers::NONE)),
            Some(Action::Quit)
        );
    }
}
