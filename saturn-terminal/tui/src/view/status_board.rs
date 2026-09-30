//! 상태판. 실행 줄, 판단 줄, 학습 줄, 대기 줄, 보류 줄, 알림 줄을 이 순서로 쌓는다.
//!
//! 설계: docs/design/tui.md(상태판 줄 순서, 상태 표시), docs/design/input-handling.md(대기와 취소, 멈춤과 보류).
//! - 같은 종류 안에서는 접수 순서(`seq`)를 따른다. 줄이 생기거나 사라져도 남은 줄끼리 상대 위치는 유지한다.
//! - 보류 줄을 뺀 나머지 줄은 그 항목이 끝나면 지운다.
//! - 판단 줄은 판단 방식과 관계없이 같은 문구를 쓰고 근거와 확률은 보이지 않는다.
//! - 대기 줄과 보류 줄의 버튼은 전체 화면에서 클릭할 수 있다(`buttons`로 칸을 구해 마우스 위치와 맞춘다).
//! 줄은 매 프레임 `build`로 `state::ChatState`에서 새로 만든다.

use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::layout::Rect;
use saturn_protocol::event::Activity;
use saturn_protocol::ids::{InputId, Provider, TaskId, TaskLabel};
use saturn_protocol::rpc::Alert;
use saturn_protocol::state::QueueReason;

use crate::i18n::Lang;
use crate::state::{ChatState, TrainingProgress};

/// 실행 줄 `명령 실행 중` 뒤에 보이는 명령 앞 칸 수.
pub const COMMAND_PREVIEW_COLS: usize = 40;

/// 줄 종류. 값 순서가 상태판 위→아래 순서다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LineKind {
    /// 실행 줄.
    Running,
    /// 판단 줄.
    Judging,
    /// 학습 줄.
    Training,
    /// 대기 줄.
    Queued,
    /// 보류 줄(멈춤 결과 줄과 보류 닫기 확인 포함).
    Held,
    /// 알림 줄.
    Alert,
}

/// 실행 줄.
/// 출력 전: `⠙ [A] 작업 중`. 출력 뒤: `⠙ [A] claude · opus · 1분 · 하위 에이전트 2개 실행 중`처럼
/// provider · 모델 · 경과 · 하는 일(subagent가 돌면 `하위 에이전트 N개 실행 중`).
/// TODO(#50): 칸 순서를 provider와 모델 먼저로 둘지, 하는 일 먼저로 둘지, 모델 이름을 보고된 그대로 쓸지 별칭으로 쓸지
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningLine {
    /// 작업.
    pub task: TaskId,
    /// 이름표.
    pub label: TaskLabel,
    /// 마지막으로 답한 provider.
    pub provider: Option<Provider>,
    /// 보고된 모델.
    pub model: Option<String>,
    /// 경과(허가 대기 동안 멈춘 값).
    pub elapsed: Duration,
    /// 하는 일. `None`이고 출력 전이면 `작업 중`.
    pub activity: Option<Activity>,
    /// 허가 응답 대기 중(`허가 기다림`). `activity`보다 앞선다.
    pub awaiting_permission: bool,
    /// 도는 provider subagent 수. 0이 아니면 하는 일 대신 `하위 에이전트 N개 실행 중`.
    pub subagents: usize,
    /// 모델 글이 한 번이라도 왔다.
    pub has_output: bool,
}

/// 상태판 한 줄.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusLine {
    /// 실행 줄.
    Running(RunningLine),
    /// 판단 줄 `⠹ [D] 판단 중 · 배포 스크립트 정리`. 원문을 모르면 `· 원문` 칸을 뺀다.
    /// TODO(#53): 짧게 끝나는 판단의 판단 줄을 생략할지, 항상 그릴지, 지연 표시와 최소 표시 시간을 둘지
    Judging {
        input: InputId,
        label: Option<TaskLabel>,
        text: Option<String>,
    },
    /// 학습 줄 `⠼ [학습] 단계 · 채점 N건 · 경과 · Token N`.
    /// TODO(#55): `/stop`이 진행 중인 `/train`도 멈출지, 학습 전용 중지 명령을 둘지, 학습 줄에 중지 버튼을 둘지
    Training(TrainingProgress),
    /// 대기 줄 `· [C] 대기 · A 다음 · 테스트도 같이 돌려줘  [보내기] [취소]`.
    /// 이유 문구는 `queue_reason_text`. `JudgeOrder`면 `[보내기]` 없이 `[취소]`만.
    Queued {
        input: InputId,
        label: Option<TaskLabel>,
        reason: QueueReason,
        text: Option<String>,
    },
    /// 멈춤 결과 `‖ 멈춤 · [A] [C] 보류됨 · /continue 로 이어서`. 보류 줄 맨 앞.
    Stopped { held: Vec<TaskLabel> },
    /// 보류 줄(작업) `‖ [E] 보류 · codex · /continue E  [이어서] [취소]`.
    HeldTask {
        task: TaskId,
        label: TaskLabel,
        provider: Option<Provider>,
    },
    /// 보류 줄(보내지 않은 입력) `‖ [C] 보류 · 원문  [이어서] [취소]`.
    HeldInput {
        input: InputId,
        label: Option<TaskLabel>,
        text: Option<String>,
    },
    /// 보류 닫기 확인 `‖ [E] 보류를 닫을까요?`. 그 작업의 보류 줄 자리에 대신 그린다. `Enter` 종료, `Esc` 유지.
    CloseHeldConfirm { task: TaskId, label: TaskLabel },
    /// 알림 줄. 문구는 `alert_text`.
    Alert(Alert),
    /// 알림 줄 `멈춤 확인 안 됨 · N개 남음`.
    StopUnconfirmed { remaining: u32 },
    /// 알림 줄 `판단기 연결 없음 · 차례에 보냅니다`. `[보내기]`를 눌렀으나 judge 실패. TODO(#46): 알릴 메서드가 없다
    JudgeUnavailableSend,
    /// 알림 줄 `폴더 설정 오류 · 이전 설정 번호 12로 계속 · 줄 7: ...`(`SettingsApplied`의 경고).
    SettingsError { previous: u64, detail: String },
}

impl StatusLine {
    /// 줄 종류.
    pub fn kind(&self) -> LineKind {
        todo!("#92")
    }

    /// 버튼을 뺀 줄 글. `spinner`는 실행·판단·학습 줄 머리 글자, 대기 줄은 `·`, 보류 줄은 `‖`.
    pub fn text(&self, lang: Lang, labels_visible: bool, spinner: char) -> String {
        todo!("#92")
    }

    /// 줄 끝 버튼. 대기 줄 `[보내기] [취소]`(`JudgeOrder`면 `[취소]`만), 보류 줄 `[이어서] [취소]`, 그 밖에는 없음.
    pub fn buttons(&self) -> Vec<Button> {
        todo!("#92")
    }
}

/// 상태판 버튼. 누르면 같은 뜻의 명령과 같다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    /// `[보내기]` = `/send` → `Request::SendNow`.
    Send(InputId),
    /// 대기 줄 `[취소]` = `/cancel` → `Request::CancelInput`.
    CancelInput(InputId),
    /// 보류 줄 `[이어서]` = `/continue` → `Request::Continue { task: Some }`.
    ContinueTask(TaskId),
    /// 보류 입력 줄 `[이어서]`. 보류 입력만 재개하는 요청이 없어 채팅 전체 `Continue { task: None }`을 쓴다(가정).
    ContinueInput(InputId),
    /// 보류 줄 `[취소]` = `/cancel` → 보류 닫기 확인 줄을 띄운다.
    CloseHeld(TaskId),
    /// 보류 입력 줄 `[취소]` → `Request::CancelInput`.
    CancelHeldInput(InputId),
}

impl Button {
    /// 버튼 글(`[보내기]`, `[취소]`, `[이어서]`).
    pub fn text(self, lang: Lang) -> &'static str {
        todo!("#92")
    }
}

/// 상태에서 줄을 만든다. 종류 순서, 같은 종류 안 접수 순서로 정렬한다.
/// 끝난 작업·끝 상태 입력은 줄이 없다. `Queued`인데 `reason`이 없으면 줄을 만들지 않는다(engine이 항상 싣는다).
pub fn build(state: &ChatState, now: Instant) -> Vec<StatusLine> {
    todo!("#92")
}

/// 대기 이유 문구: `AfterTask(A)` → `A 다음`, `JudgeOrder` → `판단 차례`, `JudgeConnection` → `판단기 연결 기다림`,
/// `WriteTurn` → `쓰기 차례`, `AfterCompaction` → `맥락 정리 뒤`, `AfterAllTasks` → `모든 작업 뒤`.
pub fn queue_reason_text(lang: Lang, reason: QueueReason) -> String {
    todo!("#92")
}

/// 실행 줄 하는 일 문구: `Thinking` → `생각 중`, `ReadingFile` → `파일 읽는 중`, `EditingFile` → `파일 수정 중`,
/// `RunningCommand` → `명령 실행 중 ` + 명령 앞 40칸, `Compacting` → `맥락 정리 중`, `SwitchingProvider` → `공급자 전환 중`.
pub fn activity_text(lang: Lang, activity: &Activity) -> String {
    todo!("#92")
}

/// 알림 문구: `JudgePaused` → `자동 판단 일시 중단`, `IntakeStopped` → `새 입력 접수 중단 · 판단기 연결을 확인하세요`,
/// `SteerNotReady(codex)` → `바로 반영: 준비 중 (codex)`, `ChatBusyElsewhere` → `다른 Saturn에서 실행 중`.
pub fn alert_text(lang: Lang, alert: &Alert) -> String {
    todo!("#92")
}

/// 줄마다 버튼 칸. `area`는 상태판 칸, 줄 `i`는 `area.y + i`행, 버튼은 줄 끝에 두 칸 띄어 오른쪽에 붙인다.
/// 그리기와 마우스 클릭 판정이 같은 계산을 쓴다.
pub fn button_rects(lines: &[StatusLine], lang: Lang, area: Rect) -> Vec<(Rect, Button)> {
    todo!("#92")
}

/// 상태판 그리기.
#[derive(Debug)]
pub struct StatusBoardView<'a> {
    /// `build`로 만든 줄.
    pub lines: &'a [StatusLine],
    /// 화면 언어.
    pub lang: Lang,
    /// 이름표를 보일지.
    pub labels_visible: bool,
    /// 스피너 글자.
    pub spinner: char,
}

impl StatusBoardView<'_> {
    /// 줄을 위에서부터 그리고 버튼을 `button_rects` 자리에 그린다. subagent 줄은 부모 줄 아래 흐리게.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        todo!("#92")
    }
}
