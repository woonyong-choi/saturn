//! 허가 요청 창. 도착한 순서대로 한 번에 하나씩 뜬다.
//!
//! 설계: docs/design/tui.md(허가 요청 창, 키), docs/design/engine-lifecycle.md(TUI가 없는 동안 보관, 다시 붙으면 가장 먼저).
//! - 제목에 작업 이름표와 provider, 본문에 요청 내용과 이유, 선택지 네 개, 허가를 기다리는 다른 작업 수를 보인다.
//! - 창이 뜬 뒤 1초 동안 키 입력을 받지 않는다(모르고 누른 키로 허가하지 않게).
//! - 여러 TUI가 붙어 있을 때 다른 클라이언트가 먼저 답하면(`Notification::PermissionResolved`) 창을 지운다.
//! - `Esc`(다르게 하라고 말하기) 뒤 입력. TODO(#56): 접두 초안으로 받을지, 창 안 입력칸으로 받을지, judge에 맡길지

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::layout::Rect;
use saturn_protocol::ids::{Provider, TaskId, TaskLabel};
use saturn_protocol::rpc::PermissionAnswer;

use crate::i18n::Lang;

/// 창이 뜬 뒤 키 입력을 받지 않는 시간.
pub const INPUT_GUARD: Duration = Duration::from_secs(1);

/// 허가 요청 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRequest {
    /// 답할 때 돌려줄 id.
    pub request_id: String,
    /// 요청한 작업.
    pub task: TaskId,
    /// 제목 이름표. 창 제목에는 이름표 보임 규칙과 관계없이 늘 붙인다.
    pub label: TaskLabel,
    /// 제목 provider.
    pub provider: Option<Provider>,
    /// 요청 내용.
    pub summary: String,
    /// 이유.
    pub reason: String,
}

/// 허가 요청 대기열. 맨 앞이 떠 있는 창.
#[derive(Debug, Default)]
pub struct PermissionQueue {
    queue: VecDeque<PermissionRequest>,
    shown_at: Option<Instant>,
}

impl PermissionQueue {
    /// 빈 대기열.
    pub fn new() -> Self {
        Self::default()
    }

    /// 요청을 끝에 더한다. 같은 `request_id`가 있으면 무시한다. 대기열이 비어 있었으면 `now`에 창이 뜬다.
    pub fn push(&mut self, request: PermissionRequest, now: Instant) {
        todo!("#92")
    }

    /// 떠 있는 요청.
    pub fn current(&self) -> Option<&PermissionRequest> {
        todo!("#92")
    }

    /// 창이 뜬 뒤 1초가 지나 키를 받을 수 있는지.
    pub fn accepts_input(&self, now: Instant) -> bool {
        todo!("#92")
    }

    /// 떠 있는 요청에 답한다. 1초 보호 중이면 `None`. 받으면 창을 내리고 다음 요청을 `now`에 띄운 뒤 보낼 답을 돌려준다.
    pub fn answer(
        &mut self,
        answer: PermissionAnswer,
        now: Instant,
    ) -> Option<(String, PermissionAnswer)> {
        todo!("#92")
    }

    /// 다른 클라이언트가 답한 요청을 지운다. 떠 있던 창이면 다음 요청을 `now`에 띄운다.
    pub fn resolve(&mut self, request_id: &str, now: Instant) {
        todo!("#92")
    }

    /// 떠 있는 요청 뒤에 허가를 기다리는 다른 작업 수.
    pub fn others_waiting(&self) -> usize {
        todo!("#92")
    }

    /// 대기열이 비었다.
    pub fn is_empty(&self) -> bool {
        todo!("#92")
    }
}

/// 허가 요청 창 그리기.
#[derive(Debug)]
pub struct PermissionView<'a> {
    /// 대기열.
    pub queue: &'a PermissionQueue,
    /// 화면 언어.
    pub lang: Lang,
    /// 1초 보호 중이면 선택지를 흐리게.
    pub guarded: bool,
}

impl PermissionView<'_> {
    /// 제목 `[A] codex`, 요청 내용, 이유, 선택지 `y 실행`, `a 이 작업 동안 같은 명령 허용`, `d 실행하지 않고 계속`,
    /// `Esc 실행하지 않고 다르게 하라고 말하기`, 다른 작업 수(0이면 생략)를 그린다.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        todo!("#92")
    }
}
