//! 대화 기록 영역. 입력 에코, 도구 셀, 결과 줄, 한 줄 알림, 피드백 질문, 이번 요청 합계를 쌓는다.
//!
//! 설계: docs/design/tui.md(영역 대화 기록, 피드백 질문, 상태 표시).
//! 갱신 시점: 판단 확정(에코), 작업 종료(작업별 출력 칸 내용과 결과 머리줄), 다시 실행할 때 기록 저장소에서 최근 부분부터 로드,
//! 위로 스크롤할 때 이전 부분 로드(`Request::LoadHistory` → `Notification::HistoryChunk`).
//! 각 셀의 글은 `TranscriptCell::lines`가 만들고 plain 출력도 같은 함수를 쓴다.

use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::Rect;
use saturn_protocol::event::Activity;
use saturn_protocol::ids::{Provider, TaskLabel};
use saturn_protocol::rpc::ChatNotice;

use crate::i18n::Lang;
use crate::shell::ShellOutput;
use crate::view::start_screen::StartInfo;

/// 입력 에코 뒤에 붙는 전달 상태 표시.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryBadge {
    /// `전달 중` 에이전트로 전달 중, 취소 불가.
    Delivering,
    /// `반영됨` 전달 완료, 취소 불가.
    Applied,
}

/// 대화 기록 한 칸.
#[derive(Debug, Clone, PartialEq)]
pub enum TranscriptCell {
    /// 시작 화면이 첫 결과 뒤 바뀐 맨 위 머리 셀.
    Header(StartInfo),
    /// 판단이 끝난 입력의 에코 `> [A] 원문`. 끼워 넣은 입력은 합쳐진 작업의 이름표.
    InputEcho {
        label: Option<TaskLabel>,
        text: String,
        badge: Option<DeliveryBadge>,
    },
    /// 모델 글. 작업이 끝날 때 작업별 출력 칸에서 옮겨 온다.
    AgentText {
        label: Option<TaskLabel>,
        lines: Vec<String>,
    },
    /// 도구 셀. 기본은 한 줄로 줄이고 전체 기록(`Ctrl+T`)에서 펼친다.
    Tool {
        label: Option<TaskLabel>,
        activity: Activity,
        output: String,
    },
    /// 결과 머리줄 `[A] codex · 45초 · Token 3,210`. 토큰 보고 전이면 `Token -`.
    Result {
        label: Option<TaskLabel>,
        provider: Option<Provider>,
        elapsed: Duration,
        tokens: Option<u64>,
    },
    /// 실패 `[A] codex · 45초 · 실패`, 다음 줄에 원인 한 줄.
    Failed {
        label: Option<TaskLabel>,
        provider: Option<Provider>,
        elapsed: Duration,
        cause: String,
    },
    /// 결과 불명 `[A] 결과 확인 필요 · /continue A`. 보류 줄과 함께 보인다.
    NeedsCheck { label: TaskLabel },
    /// `ChatNotice` 한 줄: `[A] 맥락 정리 후 이어서 진행`, `[A] codex → claude로 전환`,
    /// `이번 요청 · codex Token 4,120 · 판단기 3회 Token 9,870 · 2분 31초`.
    /// `Stopped`, `StopUnconfirmed`는 상태판에 그리고 여기에는 넣지 않는다.
    Notice {
        label: Option<TaskLabel>,
        notice: ChatNotice,
    },
    /// 피드백 질문 `[A]에 이어서 보냈어요 · 판단이 맞았나요? (선택)  1 맞아요  2 아니에요  0 닫기`. 입력 에코 다음 줄에 뜬다.
    Feedback { label: TaskLabel },
    /// 바로잡기 제안 `[B] 바로 새 작업으로 실행할까요? [실행] [그대로]`. `2` 답 뒤 입력이 아직 보내지지 않았을 때.
    Correction { label: TaskLabel },
    /// 보류 종료 완료 `[E] 보류를 닫았습니다`.
    HeldClosed { label: TaskLabel },
    /// `/train` 불가 `채점할 판단 83 / 200건 · 200건이 쌓이면 실행할 수 있습니다`.
    TrainShort { graded: u32, need: u32 },
    /// `!` 셸 명령 결과.
    Shell(ShellOutput),
    /// 한 줄 경고(명령 해석 오류 등). 원문 그대로.
    Warning(String),
}

impl TranscriptCell {
    /// 셀의 글 줄. `labels_visible`이 거짓이면 이름표를 빼고, `expanded`면 도구 셀 전체를 펼친다.
    /// 전체 화면과 plain 출력이 같은 결과를 내도록 문구는 모두 여기서 만든다.
    pub fn lines(&self, lang: Lang, labels_visible: bool, expanded: bool) -> Vec<String> {
        todo!("#92")
    }
}

/// 대화 기록 모음과 스크롤 위치.
#[derive(Debug, Default)]
pub struct Transcript {
    cells: Vec<TranscriptCell>,
    scroll_from_bottom: usize,
}

impl Transcript {
    /// 빈 기록.
    pub fn new() -> Self {
        Self::default()
    }

    /// 셀을 끝에 더한다. 맨 아래를 보고 있으면 계속 맨 아래를 따라간다.
    pub fn push(&mut self, cell: TranscriptCell) {
        todo!("#92")
    }

    /// 맨 위 머리 셀을 넣거나 바꾼다(시작 화면 전환).
    pub fn set_header(&mut self, info: StartInfo) {
        todo!("#92")
    }

    /// 떠 있는 피드백 질문 셀을 지운다(답, `0`, 8초 경과).
    pub fn remove_feedback(&mut self, label: TaskLabel) {
        todo!("#92")
    }

    /// 위로 `rows`줄 스크롤. 맨 위에 닿으면 이전 부분 로드가 필요하다고 참을 돌려준다.
    pub fn scroll_up(&mut self, rows: usize) -> bool {
        todo!("#92")
    }

    /// 아래로 `rows`줄 스크롤.
    pub fn scroll_down(&mut self, rows: usize) {
        todo!("#92")
    }

    /// 모든 셀.
    pub fn cells(&self) -> &[TranscriptCell] {
        &self.cells
    }
}

/// 대화 기록 그리기.
#[derive(Debug)]
pub struct TranscriptView<'a> {
    /// 그릴 기록.
    pub transcript: &'a Transcript,
    /// 화면 언어.
    pub lang: Lang,
    /// 이름표를 보일지.
    pub labels_visible: bool,
}

impl TranscriptView<'_> {
    /// 아래에서부터 칸 높이만큼 셀 줄을 그린다(도구 셀은 줄인 형태). 에코는 `>` 접두, 실패 원인과 subagent는 흐리게.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        todo!("#92")
    }
}
