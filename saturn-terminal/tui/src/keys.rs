//! 키 영역과 동작 이름. 키를 동작으로 바꾸는 일은 `crate::keymap`이 맡는다.
//! 설계: docs/design/tui.md

use saturn_protocol::rpc::{PermissionAnswer, UsageRange};

use crate::view::popup::PopupKind;

/// 키를 받는 영역. 영역마다 키 표가 따로 있다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum KeyArea {
    /// 모든 영역이 먼저 보는 표.
    Global,
    /// 영역의 표와 영역이 넘긴 표에 없을 때 마지막으로 보는 표.
    Fallback,
    /// 피드백 질문이 입력창이 빈 동안 키를 가져간다.
    Transcript,
    /// 바로잡기 제안이 입력창이 빈 동안 키를 가져간다.
    Correction,
    StatusBoard,
    /// 사용자가 상태판 버튼 고르기에 들어간 동안 방향키, `Enter`, `Esc`를 가져간다.
    BoardFocus,
    Popup,
    Composer,
    /// 입력 기록 검색 중. 글자는 검색어이고 나머지는 입력창 표로 넘긴다.
    Search,
    RouterKeyPrompt,
    FolderTrust,
    ResumePrompt,
    ExitConfirm,
    StopConfirm,
    /// 제약 등록 확인 창.
    ConstraintAsk,
    Permission,
    /// 입력 요청 창. 글자는 칸 입력이다.
    Input,
    TaskList,
    /// 작업 목록에서 이름이나 묶음을 쓰는 중. 글자는 그 줄 입력이다.
    TaskListEdit,
    FullTranscript,
    Usage,
    RouterVersion,
    TrainConfirm,
    ModelPicker,
    PruneWindow,
    /// 시작할 채팅 고르기 화면.
    ChatPicker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct KeyContext {
    /// 붙여넣은 요소도 없어야 참.
    pub composer_empty: bool,
    pub at_line_start: bool,
    pub at_word_start: bool,
    /// 입력창이 비었거나 불러온 기록 그대로다.
    pub history_browsable: bool,
    pub running: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Action {
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

    // 대화 기록(바로잡기 제안)
    CorrectionRun,
    CorrectionKeep,

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
    /// 빈 입력창에서도 입력창 이동일 뿐이다. 작업 목록은 `OpenTaskList`로만 연다.
    CursorLeft,
    CursorRight,
    LineStart,
    LineEnd,
    KillToStart,
    DeleteWordBack,
    RecallLatestInput,
    /// 열린 창과 초안을 먼저 정리하고, 없으면 작업을 멈추고, 멈출 것이 없으면 종료 확인(두 번 눌러 종료).
    Interrupt,
    /// `Interrupt`와 같지만 멈출 것이 없으면 바로 종료한다.
    InterruptQuit,
    /// 실행 중인 작업을 모두 멈추고 보류한다. 닫을 창이 있으면 창이 먼저 닫힌다.
    StopWork,
    /// 구현 전. 키와 명령 이름만 잡아 두었다.
    Rewind,
    CompleteCommand,
    CyclePermissionMode,
    EnterBoard,
    OpenTaskList,
    Redraw,
    ExternalEditor,
    KillToEnd,
    HistorySearch,
    Yank,
    Submit,
    SubmitQueued,
    ClearSelection,
    HistoryPrev,
    HistoryNext,

    // 입력 요청 창
    FormDecline,
    NextField,
    PrevField,

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

    // 기록 정리 창
    PruneConfirm,

    // 모델 선택 창
    SetDefaultModel,
    ToggleModelMode,

    // router 버전 화면
    ResetThresholds,
    TrainFrom,
    UseVersion,
}
