//! 제약 확인 창. 이 말을 앞으로 지킬 제약으로 등록할지 묻는다. 작업과 입력 처리는 계속된다.
//! 설계: docs/design/tui.md

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use saturn_protocol::ids::ConstraintAskId;
use saturn_protocol::rpc::ConstraintAskAnswer;

use crate::i18n::{self, Lang};
use crate::view::{MUTED, SELECTED, render_window};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AskChoice {
    Register,
    Decline,
}

impl AskChoice {
    pub(crate) fn answer(self) -> ConstraintAskAnswer {
        match self {
            Self::Register => ConstraintAskAnswer::Yes,
            Self::Decline => ConstraintAskAnswer::No,
        }
    }
}

/// 답을 기다리는 등록 확인 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConstraintAsk {
    pub ask: ConstraintAskId,
    /// 등록할 규칙. 여러 건이면 줄바꿈으로 이어 온다.
    pub rule: String,
    pub selected: AskChoice,
}

impl ConstraintAsk {
    /// 지키는 쪽인 `등록`이 처음 선택이다.
    pub(crate) fn new(ask: ConstraintAskId, rule: String) -> Self {
        Self {
            ask,
            rule,
            selected: AskChoice::Register,
        }
    }

    pub(crate) fn up(&mut self) {
        self.selected = AskChoice::Register;
    }

    pub(crate) fn down(&mut self) {
        self.selected = AskChoice::Decline;
    }
}

/// 도착 순서대로 한 번에 하나씩 띄운다. `Esc`로 미룬 확인은 큐에 남지만 다시 붙기 전에는 띄우지 않는다.
#[derive(Debug, Default)]
pub(crate) struct ConstraintAskQueue {
    pending: Vec<(ConstraintAsk, bool)>,
}

impl ConstraintAskQueue {
    /// 이미 있는 확인이면 더하지 않는다. 같은 확인은 붙을 때 다시 올 수 있다.
    pub(crate) fn push(&mut self, ask: ConstraintAsk) {
        if self.pending.iter().all(|(known, _)| known.ask != ask.ask) {
            self.pending.push((ask, false));
        }
    }

    /// 답을 받았거나 닫힌 확인을 뺀다.
    pub(crate) fn remove(&mut self, ask: ConstraintAskId) {
        self.pending.retain(|(known, _)| known.ask != ask);
    }

    /// 답을 미룬다. 확인은 큐에 남는다.
    pub(crate) fn defer(&mut self, ask: ConstraintAskId) {
        if let Some((_, deferred)) = self.pending.iter_mut().find(|(known, _)| known.ask == ask) {
            *deferred = true;
        }
    }

    /// 지금 띄울 확인. 미룬 확인은 건너뛴다.
    pub(crate) fn next(&self) -> Option<ConstraintAsk> {
        self.pending
            .iter()
            .find(|(_, deferred)| !deferred)
            .map(|(ask, _)| ask.clone())
    }

    /// `ask`를 뺀 답 기다리는 확인 수. 미룬 확인은 세지 않는다.
    pub(crate) fn waiting_besides(&self, ask: ConstraintAskId) -> usize {
        self.pending
            .iter()
            .filter(|(known, deferred)| !deferred && known.ask != ask)
            .count()
    }
}

#[derive(Debug)]
pub(crate) struct ConstraintAskView<'a> {
    pub ask: &'a ConstraintAsk,
    /// 이 확인 뒤에 답을 기다리는 다른 확인 수.
    pub waiting: usize,
    pub lang: Lang,
}

impl ConstraintAskView<'_> {
    // cost: time O(m), heap O(m), stack O(1)
    // vars: m = 규칙 글자 수
    // basis: estimate
    pub(crate) fn render(&self, frame: &mut Frame, area: Rect) {
        let lang = self.lang;
        let mut lines: Vec<Line> = self
            .ask
            .rule
            .lines()
            .map(|line| Line::from(line.to_string()))
            .collect();
        lines.push(Line::from(""));
        for (choice, text) in [
            (AskChoice::Register, i18n::CONSTRAINT_ASK_REGISTER),
            (AskChoice::Decline, i18n::CONSTRAINT_ASK_DECLINE),
        ] {
            let is_selected = choice == self.ask.selected;
            let (marker, style) = if is_selected {
                ("› ", SELECTED)
            } else {
                ("  ", Style::new())
            };
            lines.push(Line::from(Span::styled(
                format!("{marker}{}", lang.tr(text)),
                style,
            )));
        }
        lines.push(Line::from(""));
        if self.waiting > 0 {
            lines.push(Line::from(Span::styled(
                format!("{} {}", lang.tr(i18n::CONSTRAINT_ASK_WAITING), self.waiting),
                MUTED,
            )));
        }
        lines.push(Line::from(Span::styled(
            lang.tr(i18n::CONSTRAINT_ASK_HINT),
            MUTED,
        )));
        render_window(frame, area, lang.tr(i18n::CONSTRAINT_ASK_TITLE), lines);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::view::buffer_lines;

    fn ask(id: u64, rule: &str) -> ConstraintAsk {
        ConstraintAsk::new(ConstraintAskId(id), rule.to_string())
    }

    #[test]
    fn constraint_ask_starts_on_the_keeping_choice_and_moves_with_the_arrows() {
        let mut ask = ask(1, "answer in English");

        assert_eq!(ask.selected.answer(), ConstraintAskAnswer::Yes);
        ask.down();
        assert_eq!(ask.selected.answer(), ConstraintAskAnswer::No);
        ask.up();
        assert_eq!(ask.selected.answer(), ConstraintAskAnswer::Yes);
    }

    #[test]
    fn constraint_ask_queue_shows_one_at_a_time_in_arrival_order_and_skips_deferred() {
        let mut queue = ConstraintAskQueue::default();
        queue.push(ask(1, "first"));
        queue.push(ask(2, "second"));
        queue.push(ask(1, "first again"));

        assert_eq!(queue.next().unwrap().rule, "first");
        assert_eq!(queue.waiting_besides(ConstraintAskId(1)), 1);
        queue.defer(ConstraintAskId(1));
        assert_eq!(queue.next().unwrap().rule, "second");
        queue.remove(ConstraintAskId(2));
        assert_eq!(queue.next(), None);
    }

    #[test]
    fn render_shows_the_rule_both_choices_and_the_waiting_count() {
        let ask = ask(1, "answer in English");
        let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();

        terminal
            .draw(|frame| {
                ConstraintAskView {
                    ask: &ask,
                    waiting: 2,
                    lang: Lang::En,
                }
                .render(frame, frame.area());
            })
            .unwrap();

        let text = buffer_lines(terminal.backend().buffer()).join("\n");
        assert!(text.contains("Add this as a constraint?"), "{text}");
        assert!(text.contains("answer in English"), "{text}");
        assert!(text.contains("Add"), "{text}");
        assert!(text.contains("Don't add"), "{text}");
        assert!(text.contains("2"), "{text}");
    }
}
