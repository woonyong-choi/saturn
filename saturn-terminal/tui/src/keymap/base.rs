//! 기본 키 표. 네 도구(Claude Code, Codex, Gemini CLI, OpenCode)가 같은 뜻으로 쓰는 키와, 뜻이 갈리는 키 중 더 많은
//! 도구가 쓰는 뜻을 담는다. 프리셋은 이 표에서 다른 줄만 덮어쓴다.
//! 설계: docs/design/tui.md

use saturn_protocol::rpc::{PermissionAnswer, UsageRange};

use super::{Binding, Cond};
use crate::keys::Action;
use crate::keys::KeyArea as A;

fn b(action: Action, scopes: &[A], keys: &[&'static str]) -> Binding {
    Binding::new(action, scopes, keys)
}

/// 목록을 `↑`, `↓`로 고르는 영역.
const LISTS: &[A] = &[
    A::BoardFocus,
    A::Popup,
    A::FolderTrust,
    A::ResumePrompt,
    A::ExitConfirm,
    A::StopConfirm,
    A::ConstraintAsk,
    A::TaskList,
    A::FullTranscript,
    A::RouterVersion,
    A::TrainConfirm,
    A::ModelPicker,
    A::PruneWindow,
    A::ChatPicker,
    A::Transcript,
    A::Correction,
    A::Input,
];

/// `Enter`로 정하는 영역.
const CONFIRMS: &[A] = &[
    A::BoardFocus,
    A::Popup,
    A::RouterKeyPrompt,
    A::FolderTrust,
    A::ResumePrompt,
    A::ExitConfirm,
    A::StopConfirm,
    A::ConstraintAsk,
    A::TaskList,
    A::Usage,
    A::RouterVersion,
    A::TrainConfirm,
    A::ModelPicker,
    A::ChatPicker,
    A::Transcript,
    A::Correction,
    A::Search,
    A::Input,
];

/// `Esc`로 닫는 영역.
const CLOSES: &[A] = &[
    A::BoardFocus,
    A::Popup,
    A::ExitConfirm,
    A::StopConfirm,
    A::ConstraintAsk,
    A::TaskList,
    A::FullTranscript,
    A::Usage,
    A::RouterVersion,
    A::TrainConfirm,
    A::ModelPicker,
    A::PruneWindow,
    A::ChatPicker,
    A::Transcript,
    A::Correction,
    A::Search,
    A::Input,
];

/// 영역마다 같은 뜻으로 쓰는 키(`↑`, `↓`, `Enter`, `Esc`)와 영역별 키, 입력창 키를 모두 담는다.
pub(super) fn table() -> Vec<Binding> {
    let mut table = vec![
        // 모든 영역
        b(Action::ShowFullTranscript, &[A::Global], &["ctrl+t"]).command("/transcript"),
        b(Action::Suspend, &[A::Global], &["ctrl+z"]).command("/suspend"),
        // 영역의 표에도 입력창의 표에도 없을 때
        b(Action::Interrupt, &[A::Fallback], &["ctrl+c"]).command("/stop"),
        // 목록과 선택지
        b(Action::Up, LISTS, &["up"]),
        b(Action::Down, LISTS, &["down"]),
        b(Action::Confirm, CONFIRMS, &["enter"]),
        b(Action::Close, CLOSES, &["esc"]),
        // 피드백 질문과 바로잡기 제안
        b(Action::FeedbackAnswer(true), &[A::Transcript], &["1"]),
        b(Action::FeedbackAnswer(false), &[A::Transcript], &["2"]),
        b(Action::FeedbackDismiss, &[A::Transcript], &["0"]),
        b(Action::CorrectionRun, &[A::Correction], &["1"]),
        b(Action::CorrectionKeep, &[A::Correction], &["2"]),
        // 상태판
        b(Action::ToggleDetail, &[A::Detail], &["enter"]),
        b(Action::ConfirmCloseHeld, &[A::StatusBoard], &["enter"]),
        b(Action::KeepHeld, &[A::StatusBoard], &["esc"]),
        // 팝업
        b(Action::PopupComplete, &[A::Popup], &["tab"]),
        // router 키 입력 창
        b(Action::Quit, &[A::RouterKeyPrompt], &["esc"]),
        b(
            Action::Backspace,
            &[A::RouterKeyPrompt, A::Search, A::TaskListEdit, A::Input],
            &["backspace"],
        ),
        // 폴더 설정 신뢰 창
        b(
            Action::Quit,
            &[A::FolderTrust],
            &["esc", "ctrl+c", "3", "q"],
        ),
        b(Action::TrustApply, &[A::FolderTrust], &["1", "y"]),
        // 종료 확인 창, 멈춤 확인 창, 제약 확인 창
        b(
            Action::Close,
            &[A::ExitConfirm, A::StopConfirm, A::ConstraintAsk],
            &["ctrl+c"],
        ),
        // 허가 요청 창
        b(
            Action::Permission(PermissionAnswer::AllowOnce),
            &[A::Permission],
            &["y"],
        ),
        b(
            Action::Permission(PermissionAnswer::AllowAlways),
            &[A::Permission],
            &["a"],
        ),
        b(
            Action::Permission(PermissionAnswer::Deny { note: None }),
            &[A::Permission],
            &["d", "esc"],
        ),
        // 입력 요청 창
        b(Action::FormDecline, &[A::Input], &["ctrl+d"]),
        b(Action::NextField, &[A::Input], &["tab"]),
        b(Action::PrevField, &[A::Input], &["shift+tab"]),
        // 작업 목록 화면
        b(Action::NextFilter, &[A::TaskList], &["tab"]),
        b(Action::PrevFilter, &[A::TaskList], &["shift+tab"]),
        b(Action::Help, &[A::TaskList], &["?"]),
        b(Action::ToggleFolderScope, &[A::TaskList], &["a"]),
        b(Action::ContinueHeld, &[A::TaskList], &["c"]),
        b(Action::CancelOrCloseHeld, &[A::TaskList], &["d"]),
        b(Action::Search, &[A::TaskList], &["f"]),
        b(Action::ChangeGroup, &[A::TaskList], &["g"]),
        b(Action::NewChat, &[A::TaskList], &["n"]),
        b(Action::RenameChat, &[A::TaskList], &["r"]),
        b(Action::SendQueued, &[A::TaskList], &["s"]),
        // 사용량 화면
        b(Action::UsageRange(UsageRange::Day), &[A::Usage], &["d"]),
        b(Action::UsageRange(UsageRange::Week), &[A::Usage], &["w"]),
        // router 버전 화면
        b(Action::ResetThresholds, &[A::RouterVersion], &["r"]),
        b(Action::TrainFrom, &[A::RouterVersion], &["t"]),
        b(Action::UseVersion, &[A::RouterVersion], &["u"]),
        // 기록 정리 창
        b(Action::PruneConfirm, &[A::PruneWindow], &["y"]),
        b(Action::Close, &[A::PruneWindow], &["ctrl+c"]),
        // 모델 선택 창
        b(Action::SetDefaultModel, &[A::ModelPicker], &["d"]),
        b(Action::ToggleModelMode, &[A::ModelPicker], &["m"]),
        // 시작할 채팅 고르기
        b(Action::Close, &[A::ChatPicker], &["ctrl+c"]),
    ];
    table.extend(composer());
    table
}

/// 입력창 키. `Esc`와 `Tab`, `Shift+Tab`, `Ctrl+C`처럼 도구마다 뜻이 갈리는 키가 여기 있다.
fn composer() -> Vec<Binding> {
    let c = &[A::Composer];
    vec![
        b(Action::Submit, c, &["enter"]),
        b(Action::SubmitQueued, c, &["ctrl+q"]),
        b(Action::Newline, c, &["ctrl+j", "shift+enter", "alt+enter"]),
        b(Action::HistoryPrev, c, &["up"]).when(Cond::HistoryBrowsable),
        b(Action::HistoryNext, c, &["down"]).when(Cond::HistoryBrowsable),
        b(Action::Up, c, &["up"]),
        b(Action::Down, c, &["down"]),
        b(Action::Backspace, c, &["backspace"]),
        b(Action::CursorLeft, c, &["left"]),
        b(Action::CursorRight, c, &["right"]),
        b(Action::LineStart, c, &["ctrl+a", "home"]),
        b(Action::LineEnd, c, &["ctrl+e", "end"]),
        b(Action::KillToEnd, c, &["ctrl+k"]),
        b(Action::KillToStart, c, &["ctrl+u"]),
        b(Action::DeleteWordBack, c, &["ctrl+w", "alt+backspace"]),
        b(Action::Yank, c, &["ctrl+y"]),
        b(Action::HistorySearch, c, &["ctrl+r"]),
        b(Action::ExternalEditor, c, &["ctrl+g"]),
        b(Action::RecallLatestInput, c, &["alt+up", "shift+left"]),
        b(Action::Quit, c, &["ctrl+d"])
            .when(Cond::ComposerEmpty)
            .command("/quit"),
        b(Action::Redraw, c, &["ctrl+l"]).command("/redraw"),
        // 뜻이 갈리는 키
        b(Action::StopWork, c, &["esc"]).command("/stop"),
        b(Action::Rewind, c, &["esc esc"]).command("/rewind"),
        b(Action::CyclePermissionMode, c, &["shift+tab"]).command("/mode"),
        b(Action::CompleteCommand, c, &["tab"]),
        // Saturn 화면 키
        b(Action::EnterBoard, c, &["f3"]).command("/agents"),
        b(Action::OpenTaskList, c, &["f5"]).command("/tasks"),
    ]
}
