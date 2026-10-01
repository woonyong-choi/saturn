//! 허가 요청 창. 도착한 순서대로 한 번에 하나씩 뜬다.
//! 설계: docs/design/tui.md
//! TODO(#56): `Esc`(다르게 하라고 말하기) 뒤 입력을 받는 방식

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::layout::Rect;
use saturn_protocol::ids::{Provider, TaskId, TaskLabel};
use saturn_protocol::rpc::PermissionAnswer;

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::i18n::{self, Lang};
use crate::labels;
use crate::view::{MUTED, render_window};

/// 모르고 누른 키로 허가하지 않게 막는 시간.
pub const INPUT_GUARD: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRequest {
    pub request_id: String,
    pub task: TaskId,
    /// 창 제목에는 이름표 보임 규칙과 관계없이 늘 붙인다.
    pub label: TaskLabel,
    pub provider: Option<Provider>,
    pub summary: String,
    pub reason: String,
}

/// 맨 앞이 떠 있는 창.
#[derive(Debug, Default)]
pub struct PermissionQueue {
    queue: VecDeque<PermissionRequest>,
    shown_at: Option<Instant>,
}

impl PermissionQueue {
    pub fn new() -> Self {
        Self::default()
    }

    // cost: time O(q), heap O(1), stack O(1)
    // vars: q = 대기열 길이
    // basis: estimate
    /// 같은 `request_id`가 있으면 무시한다.
    pub fn push(&mut self, request: PermissionRequest, now: Instant) {
        if self
            .queue
            .iter()
            .any(|r| r.request_id == request.request_id)
        {
            return;
        }
        if self.queue.is_empty() {
            self.shown_at = Some(now);
        }
        self.queue.push_back(request);
    }

    pub fn current(&self) -> Option<&PermissionRequest> {
        self.queue.front()
    }

    pub fn accepts_input(&self, now: Instant) -> bool {
        self.shown_at
            .is_some_and(|shown| now.saturating_duration_since(shown) >= INPUT_GUARD)
    }

    /// 보호 시간 중이면 `None`.
    pub fn answer(
        &mut self,
        answer: PermissionAnswer,
        now: Instant,
    ) -> Option<(String, PermissionAnswer)> {
        if !self.accepts_input(now) {
            return None;
        }
        let request = self.queue.pop_front()?;
        self.shown_at = (!self.queue.is_empty()).then_some(now);
        Some((request.request_id, answer))
    }

    // cost: time O(q), heap O(1), stack O(1)
    // vars: q = 대기열 길이
    // basis: estimate
    pub fn resolve(&mut self, request_id: &str, now: Instant) {
        let Some(index) = self.queue.iter().position(|r| r.request_id == request_id) else {
            return;
        };
        self.queue.remove(index);
        if index == 0 {
            self.shown_at = (!self.queue.is_empty()).then_some(now);
        }
    }

    pub fn others_waiting(&self) -> usize {
        self.queue.len().saturating_sub(1)
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

#[derive(Debug)]
pub struct PermissionView<'a> {
    pub queue: &'a PermissionQueue,
    pub lang: Lang,
    pub guarded: bool,
}

impl PermissionView<'_> {
    // cost: time O(m), heap O(m), stack O(1)
    // vars: m = 요청 내용과 이유 길이
    // basis: estimate
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let Some(request) = self.queue.current() else {
            return;
        };
        let lang = self.lang;
        let choice_style = if self.guarded { MUTED } else { Style::new() };
        let mut lines = vec![
            Line::from(request.summary.clone()),
            Line::from(format!(
                "{}: {}",
                lang.tr(i18n::PERMISSION_REASON),
                request.reason
            )),
            Line::from(""),
        ];
        lines.extend(
            [
                ("y", i18n::PERMISSION_ALLOW),
                ("a", i18n::PERMISSION_ALLOW_FOR_TASK),
                ("d", i18n::PERMISSION_DENY),
                ("Esc", i18n::PERMISSION_DENY_AND_REDIRECT),
            ]
            .into_iter()
            .map(|(key, text)| {
                Line::from(Span::styled(
                    format!("{key} {}", lang.tr(text)),
                    choice_style,
                ))
            }),
        );
        let others = self.queue.others_waiting();
        if others > 0 {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!(
                    "{} {}",
                    lang.tr(i18n::PERMISSION_WAITING),
                    i18n::format_items(lang, others as u64)
                ),
                MUTED,
            )));
        }
        let title = match request.provider {
            Some(provider) => format!(
                "{} {}",
                labels::format(request.label),
                i18n::provider_name(provider)
            ),
            None => labels::format(request.label),
        };
        render_window(frame, area, &title, lines);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn request(id: &str) -> PermissionRequest {
        PermissionRequest {
            request_id: id.to_string(),
            task: TaskId(1),
            label: TaskLabel('A'),
            provider: Some(Provider::Codex),
            summary: "rm -rf build".to_string(),
            reason: "clean build".to_string(),
        }
    }

    #[test]
    fn answer_within_guard_is_ignored() {
        let now = Instant::now();
        let mut queue = PermissionQueue::new();
        queue.push(request("r1"), now);

        let early = queue.answer(PermissionAnswer::Allow, now + Duration::from_millis(500));
        let late = queue.answer(PermissionAnswer::Allow, now + Duration::from_secs(1));

        assert_eq!(early, None);
        assert_eq!(late, Some(("r1".to_string(), PermissionAnswer::Allow)));
        assert!(queue.is_empty());
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn answer_shows_next_with_fresh_guard() {
        let now = Instant::now();
        let mut queue = PermissionQueue::new();
        queue.push(request("r1"), now);
        queue.push(request("r2"), now);
        let later = now + Duration::from_secs(2);

        queue.answer(PermissionAnswer::Deny, later);

        assert_eq!(queue.current().map(|r| r.request_id.as_str()), Some("r2"));
        assert!(!queue.accepts_input(later));
    }

    #[test]
    fn push_duplicate_is_ignored() {
        let now = Instant::now();
        let mut queue = PermissionQueue::new();

        queue.push(request("r1"), now);
        queue.push(request("r1"), now);

        assert_eq!(queue.others_waiting(), 0);
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn resolve_removes_current_and_shows_next() {
        let now = Instant::now();
        let mut queue = PermissionQueue::new();
        queue.push(request("r1"), now);
        queue.push(request("r2"), now);

        queue.resolve("r1", now);

        assert_eq!(queue.current().map(|r| r.request_id.as_str()), Some("r2"));
        queue.resolve("missing", now);
        assert_eq!(queue.others_waiting(), 0);
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn render_shows_title_choices_and_waiting_count() {
        let now = Instant::now();
        let mut queue = PermissionQueue::new();
        queue.push(request("r1"), now);
        queue.push(request("r2"), now);
        let mut terminal = Terminal::new(TestBackend::new(70, 14)).unwrap();
        let view = PermissionView {
            queue: &queue,
            lang: Lang::En,
            guarded: false,
        };

        terminal
            .draw(|frame| view.render(frame, frame.area()))
            .unwrap();

        let content: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(content.contains("[A] codex"));
        assert!(content.contains("y run"));
        assert!(content.contains("other tasks waiting for permission: 1"));
    }
}
