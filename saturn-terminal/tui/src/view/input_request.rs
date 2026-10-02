//! provider 입력 요청 창. 허가 요청 창처럼 도착한 순서대로 한 번에 하나씩 뜬다.
//! 설계: docs/design/tui.md#입력-요청-창

use std::collections::VecDeque;
use std::time::Instant;

use crossterm::event::KeyEvent;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use saturn_protocol::ids::{Provider, TaskId, TaskLabel};
use saturn_protocol::input::{InputAnswer, InputRequest};

use crate::i18n::{self, Lang};
use crate::labels;
use crate::view::input_form::{ChoiceState, FieldState, FormError, FormEvent, InputForm};
use crate::view::permission::INPUT_GUARD;
use crate::view::{EMPHASIS, MUTED, render_window, wrap};

/// 메시지를 접는 폭.
const MESSAGE_WIDTH: usize = 72;

/// 비밀 칸에서 글자 하나를 대신하는 글자.
const SECRET_MARK: char = '•';

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct InputEntry {
    pub request_id: String,
    pub task: TaskId,
    /// 창 제목에는 이름표 보임 규칙과 관계없이 늘 붙인다.
    pub label: TaskLabel,
    pub provider: Option<Provider>,
    pub form: InputForm,
}

/// 맨 앞이 떠 있는 창.
#[derive(Debug, Default)]
pub(crate) struct InputQueue {
    queue: VecDeque<InputEntry>,
    shown_at: Option<Instant>,
}

impl InputQueue {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    // cost: time O(q), heap O(f), stack O(1)
    // vars: q = 대기열 길이, f = 요청의 칸 수
    // basis: estimate
    /// 같은 `request_id`가 있으면 무시한다.
    pub(crate) fn push(
        &mut self,
        request_id: String,
        task: TaskId,
        label: TaskLabel,
        provider: Option<Provider>,
        request: InputRequest,
        now: Instant,
    ) {
        if self
            .queue
            .iter()
            .any(|entry| entry.request_id == request_id)
        {
            return;
        }
        if self.queue.is_empty() {
            self.shown_at = Some(now);
        }
        self.queue.push_back(InputEntry {
            request_id,
            task,
            label,
            provider,
            form: InputForm::new(request),
        });
    }

    pub(crate) fn current(&self) -> Option<&InputEntry> {
        self.queue.front()
    }

    /// 모르고 누른 키가 답이 되지 않게 창이 뜬 뒤 1초는 키를 받지 않는다.
    pub(crate) fn accepts_input(&self, now: Instant) -> bool {
        self.shown_at
            .is_some_and(|shown| now.saturating_duration_since(shown) >= INPUT_GUARD)
    }

    // cost: time O(f·o), heap O(f·o), stack O(1)
    // vars: f = 칸 수, o = 선택지 수
    // basis: estimate
    /// 보호 시간 중이면 `None`. 답이 정해지면 창을 닫고 요청 번호와 답을 돌려준다.
    pub(crate) fn on_key(&mut self, key: KeyEvent, now: Instant) -> Option<(String, InputAnswer)> {
        if !self.accepts_input(now) {
            return None;
        }
        let entry = self.queue.front_mut()?;
        let FormEvent::Answer(answer) = entry.form.on_key(key) else {
            return None;
        };
        let request_id = self.queue.pop_front()?.request_id;
        self.shown_at = (!self.queue.is_empty()).then_some(now);
        Some((request_id, answer))
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    pub(crate) fn paste(&mut self, text: &str, now: Instant) {
        if !self.accepts_input(now) {
            return;
        }
        if let Some(entry) = self.queue.front_mut() {
            entry.form.paste(text);
        }
    }

    // cost: time O(q), heap O(1), stack O(1)
    // vars: q = 대기열 길이
    // basis: estimate
    pub(crate) fn resolve(&mut self, request_id: &str, now: Instant) {
        let Some(index) = self
            .queue
            .iter()
            .position(|entry| entry.request_id == request_id)
        else {
            return;
        };
        self.queue.remove(index);
        if index == 0 {
            self.shown_at = (!self.queue.is_empty()).then_some(now);
        }
    }

    pub(crate) fn others_waiting(&self) -> usize {
        self.queue.len().saturating_sub(1)
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    pub(crate) fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

#[derive(Debug)]
pub(crate) struct InputView<'a> {
    pub queue: &'a InputQueue,
    pub lang: Lang,
    pub guarded: bool,
}

impl InputView<'_> {
    // cost: time O(f·o), heap O(f·o), stack O(1)
    // vars: f = 칸 수, o = 선택지 수
    // basis: estimate
    pub(crate) fn render(&self, frame: &mut Frame, area: Rect) {
        let Some(entry) = self.queue.current() else {
            return;
        };
        let lang = self.lang;
        let form = &entry.form;
        let mut lines: Vec<Line<'static>> = Vec::new();
        if !form.request.message.is_empty() {
            lines.extend(
                wrap(&form.request.message, MESSAGE_WIDTH)
                    .into_iter()
                    .map(Line::from),
            );
            lines.push(Line::from(""));
        }
        if let Some(url) = &form.request.url {
            lines.push(Line::from(Span::styled(url.clone(), EMPHASIS)));
            lines.push(Line::from(Span::styled(
                lang.tr(i18n::INPUT_LINK_NOTE),
                MUTED,
            )));
        }
        for index in 0..form.states.len() {
            lines.extend(field_lines(form, index, lang));
        }
        lines.push(Line::from(""));
        let help = if form.is_link() {
            i18n::INPUT_LINK_HELP
        } else {
            i18n::INPUT_FORM_HELP
        };
        let style = if self.guarded {
            MUTED
        } else {
            ratatui::style::Style::new()
        };
        lines.push(Line::from(Span::styled(lang.tr(help), style)));
        let others = self.queue.others_waiting();
        if others > 0 {
            lines.push(Line::from(Span::styled(
                format!(
                    "{} {}",
                    lang.tr(i18n::INPUT_WAITING),
                    i18n::format_items(lang, others as u64)
                ),
                MUTED,
            )));
        }
        let title = match entry.provider {
            Some(provider) => format!(
                "{} {}",
                labels::format(entry.label),
                i18n::provider_name(provider)
            ),
            None => labels::format(entry.label),
        };
        render_window(frame, area, &title, lines);
    }
}

// cost: time O(o), heap O(o), stack O(1), alloc o
// vars: o = 선택지 수
// basis: estimate
fn field_lines(form: &InputForm, index: usize, lang: Lang) -> Vec<Line<'static>> {
    let (Some(field), Some(state)) = (form.field(index), form.states.get(index)) else {
        return Vec::new();
    };
    let is_focused = index == form.focus;
    let marker = if is_focused { '›' } else { ' ' };
    let required = if field.is_required { " *" } else { "" };
    let mut header = vec![Span::styled(
        format!("{marker} {}{required}", field.title),
        if is_focused {
            EMPHASIS
        } else {
            ratatui::style::Style::new()
        },
    )];
    if !field.description.is_empty() {
        header.push(Span::styled(format!("  {}", field.description), MUTED));
    }
    let mut lines = vec![Line::from(header)];
    if is_focused {
        lines.extend(
            body_lines(state, field.is_secret, lang)
                .into_iter()
                .map(Line::from),
        );
    } else {
        lines.push(Line::from(Span::styled(
            format!("    {}", summary(state, field.is_secret, lang)),
            MUTED,
        )));
    }
    if let Some((_, error)) = form.error.filter(|(failed, _)| *failed == index) {
        lines.push(Line::from(Span::styled(
            format!("    {}", lang.tr(error_text(error))),
            EMPHASIS,
        )));
    }
    lines
}

fn error_text(error: FormError) -> &'static str {
    match error {
        FormError::Required => i18n::INPUT_REQUIRED,
        FormError::NotNumber => i18n::INPUT_NOT_NUMBER,
        FormError::NotInteger => i18n::INPUT_NOT_INTEGER,
    }
}

// cost: time O(o), heap O(o), stack O(1), alloc 1
// vars: o = 선택지 수
// basis: estimate
/// 칸이 접혀 있을 때 한 줄에 보이는 값. 비었으면 `-`.
fn summary(state: &FieldState, is_secret: bool, lang: Lang) -> String {
    let text = match state {
        FieldState::Text(text) | FieldState::Number { text, .. } => shown(text, is_secret),
        FieldState::Boolean(flag) => flag.map_or_else(String::new, |flag| yes_no(flag, lang)),
        FieldState::Choice(choice) => shown(&choice.picked_labels().join(", "), is_secret),
    };
    if text.is_empty() {
        "-".to_owned()
    } else {
        text
    }
}

fn shown(text: &str, is_secret: bool) -> String {
    if is_secret {
        SECRET_MARK.to_string().repeat(text.chars().count())
    } else {
        text.to_owned()
    }
}

fn yes_no(flag: bool, lang: Lang) -> String {
    lang.tr(if flag {
        i18n::INPUT_YES
    } else {
        i18n::INPUT_NO
    })
    .to_owned()
}

// cost: time O(o), heap O(o), stack O(1), alloc o
// vars: o = 선택지 수
// basis: estimate
/// 포커스가 있는 칸의 입력 줄.
fn body_lines(state: &FieldState, is_secret: bool, lang: Lang) -> Vec<String> {
    match state {
        FieldState::Text(text) | FieldState::Number { text, .. } => {
            vec![format!("    › {}▏", shown(text, is_secret))]
        }
        FieldState::Boolean(flag) => {
            let mark = |is_picked: bool| if is_picked { "(•)" } else { "( )" };
            vec![format!(
                "    {} {}   {} {}",
                mark(*flag == Some(true)),
                yes_no(true, lang),
                mark(*flag == Some(false)),
                yes_no(false, lang),
            )]
        }
        FieldState::Choice(choice) => (0..choice.rows())
            .map(|row| choice_line(choice, row, lang))
            .collect(),
    }
}

fn choice_line(choice: &ChoiceState, row: usize, lang: Lang) -> String {
    let cursor = if row == choice.cursor { '›' } else { ' ' };
    let mark = match (choice.is_multi, choice.picked[row]) {
        (true, true) => "[x]",
        (true, false) => "[ ]",
        (false, true) => "(•)",
        (false, false) => "( )",
    };
    if choice.is_other_row(row) {
        let caret = if row == choice.cursor { "▏" } else { "" };
        return format!(
            "  {cursor} {mark} {}: {}{caret}",
            lang.tr(i18n::INPUT_OTHER),
            choice.other
        );
    }
    let option = &choice.options[row];
    if option.description.is_empty() {
        format!("  {cursor} {mark} {}", option.label)
    } else {
        format!("  {cursor} {mark} {}  {}", option.label, option.description)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crossterm::event::{KeyCode, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use saturn_protocol::input::{InputField, InputFieldKind, InputOption, InputValue};

    use super::*;
    use crate::view::buffer_lines;

    fn text_field(id: &str, is_secret: bool) -> InputField {
        InputField {
            id: id.to_owned(),
            title: format!("{id} title"),
            description: "short".to_owned(),
            kind: InputFieldKind::Text,
            is_required: true,
            is_secret,
        }
    }

    fn choice_field() -> InputField {
        InputField {
            id: "pick".to_owned(),
            title: "pick one".to_owned(),
            description: String::new(),
            kind: InputFieldKind::Choice {
                options: vec![
                    InputOption {
                        value: "a".to_owned(),
                        label: "Alpha".to_owned(),
                        description: "first".to_owned(),
                    },
                    InputOption {
                        value: "b".to_owned(),
                        label: "Beta".to_owned(),
                        description: String::new(),
                    },
                ],
                allows_other: true,
            },
            is_required: false,
            is_secret: false,
        }
    }

    fn request(fields: Vec<InputField>) -> InputRequest {
        InputRequest {
            message: "fill this in".to_owned(),
            fields,
            url: None,
        }
    }

    fn push(queue: &mut InputQueue, id: &str, request: InputRequest, now: Instant) {
        queue.push(
            id.to_owned(),
            TaskId(1),
            TaskLabel('A'),
            Some(Provider::Claude),
            request,
            now,
        );
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn screen(queue: &InputQueue, lang: Lang, guarded: bool) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(90, 20)).unwrap();
        let view = InputView {
            queue,
            lang,
            guarded,
        };
        terminal
            .draw(|frame| view.render(frame, frame.area()))
            .unwrap();
        buffer_lines(terminal.backend().buffer())
    }

    #[test]
    fn elicitation_keys_are_ignored_during_the_guard_time() {
        let now = Instant::now();
        let mut queue = InputQueue::new();
        push(&mut queue, "r1", request(vec![text_field("a", false)]), now);

        let early = queue.on_key(key(KeyCode::Esc), now + Duration::from_millis(300));
        let late = queue.on_key(key(KeyCode::Esc), now + Duration::from_secs(1));

        assert_eq!(early, None);
        assert_eq!(late, Some(("r1".to_owned(), InputAnswer::Cancel)));
        assert!(queue.is_empty());
    }

    #[test]
    fn elicitation_answer_shows_the_next_request_with_a_fresh_guard() {
        let now = Instant::now();
        let mut queue = InputQueue::new();
        push(&mut queue, "r1", request(vec![text_field("a", false)]), now);
        push(&mut queue, "r2", request(vec![text_field("b", false)]), now);
        push(&mut queue, "r2", request(vec![text_field("b", false)]), now);
        let later = now + Duration::from_secs(2);

        queue.on_key(key(KeyCode::Esc), later);

        assert_eq!(queue.current().map(|e| e.request_id.as_str()), Some("r2"));
        assert!(!queue.accepts_input(later));
        queue.resolve("r2", later);
        assert!(queue.is_empty());
    }

    #[test]
    fn elicitation_typed_text_reaches_the_answer() {
        let now = Instant::now();
        let later = now + Duration::from_secs(2);
        let mut queue = InputQueue::new();
        push(&mut queue, "r1", request(vec![text_field("a", false)]), now);

        queue.paste("hi", later);
        let answer = queue.on_key(key(KeyCode::Enter), later);

        assert_eq!(
            answer,
            Some((
                "r1".to_owned(),
                InputAnswer::Submit {
                    values: vec![("a".to_owned(), InputValue::Text("hi".to_owned()))],
                }
            ))
        );
    }

    #[test]
    fn elicitation_window_shows_title_fields_choices_and_keys() {
        let now = Instant::now();
        let mut queue = InputQueue::new();
        push(
            &mut queue,
            "r1",
            request(vec![text_field("a", false), choice_field()]),
            now,
        );
        push(&mut queue, "r2", request(vec![choice_field()]), now);
        queue.on_key(key(KeyCode::Char('x')), now + Duration::from_secs(2));
        queue.on_key(key(KeyCode::Tab), now + Duration::from_secs(2));

        let text = screen(&queue, Lang::En, false).join("\n");

        assert!(text.contains("[A] claude"), "{text}");
        assert!(text.contains("fill this in"), "{text}");
        assert!(text.contains("a title *"), "{text}");
        assert!(text.contains("short"), "{text}");
        assert!(
            text.contains("(•) Alpha  first") || text.contains("( ) Alpha  first"),
            "{text}"
        );
        assert!(text.contains("( ) other:"), "{text}");
        assert!(text.contains("Ctrl+D decline"), "{text}");
        assert!(text.contains("other requests waiting: 1"), "{text}");
    }

    #[test]
    fn elicitation_window_texts_follow_the_language() {
        let now = Instant::now();
        let mut queue = InputQueue::new();
        push(&mut queue, "r1", request(vec![choice_field()]), now);

        let text = screen(&queue, Lang::Ko, false).join("\n");

        assert!(text.contains("직접 입력:"), "{text}");
        assert!(text.contains("Ctrl+D 거절"), "{text}");
    }

    #[test]
    fn elicitation_secret_text_is_masked_on_screen() {
        let now = Instant::now();
        let later = now + Duration::from_secs(2);
        let mut queue = InputQueue::new();
        push(&mut queue, "r1", request(vec![text_field("k", true)]), now);
        queue.paste("hunter2", later);

        let text = screen(&queue, Lang::En, false).join("\n");

        assert!(!text.contains("hunter2"), "{text}");
        assert!(text.contains("•••••••"), "{text}");
    }

    #[test]
    fn elicitation_url_window_shows_the_link_and_never_opens_it() {
        let now = Instant::now();
        let mut queue = InputQueue::new();
        queue.push(
            "r1".to_owned(),
            TaskId(1),
            TaskLabel('A'),
            Some(Provider::Codex),
            InputRequest {
                message: "open the page".to_owned(),
                fields: Vec::new(),
                url: Some("https://example.invalid/experiment-252".to_owned()),
            },
            now,
        );

        let text = screen(&queue, Lang::En, false).join("\n");

        assert!(
            text.contains("https://example.invalid/experiment-252"),
            "{text}"
        );
        assert!(text.contains("Saturn does not open it"), "{text}");
        assert!(text.contains("Enter continue"), "{text}");
    }
}
