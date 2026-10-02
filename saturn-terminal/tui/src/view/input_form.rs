//! provider 입력 요청의 폼 상태. 칸 종류별 입력과 검사, 답 만들기를 맡고 그리기는 `input_request`가 한다.
//! 설계: docs/design/tui.md#입력-요청-창

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use saturn_protocol::input::{
    InputAnswer, InputField, InputFieldKind, InputOption, InputRequest, InputValue,
};

/// 키를 처리한 결과. 답이 정해지면 `Answer`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum FormEvent {
    Edited,
    Answer(InputAnswer),
}

/// 제출을 막은 이유. 칸 아래에 보인다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FormError {
    Required,
    NotNumber,
    NotInteger,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChoiceState {
    pub(crate) options: Vec<InputOption>,
    pub(crate) is_multi: bool,
    pub(crate) allows_other: bool,
    /// 커서가 있는 줄. `allows_other`면 마지막 줄이 직접 입력 줄이다.
    pub(crate) cursor: usize,
    /// 줄마다 골랐는지. 길이는 줄 수와 같다.
    pub(crate) picked: Vec<bool>,
    pub(crate) other: String,
}

impl ChoiceState {
    // cost: time O(o), heap O(o), stack O(1), alloc 2
    // vars: o = 선택지 수
    // basis: estimate
    fn new(options: &[InputOption], is_multi: bool, allows_other: bool) -> Self {
        let rows = options.len() + usize::from(allows_other);
        Self {
            options: options.to_vec(),
            is_multi,
            allows_other,
            cursor: 0,
            picked: vec![false; rows],
            other: String::new(),
        }
    }

    pub(crate) fn rows(&self) -> usize {
        self.picked.len()
    }

    pub(crate) fn is_other_row(&self, row: usize) -> bool {
        self.allows_other && row == self.options.len()
    }

    /// 단일 선택은 하나만 남긴다.
    fn pick(&mut self, row: usize) {
        if !self.is_multi {
            self.picked.fill(false);
        }
        self.picked[row] = true;
    }

    fn toggle(&mut self, row: usize) {
        if self.is_multi {
            self.picked[row] = !self.picked[row];
        } else {
            self.pick(row);
        }
    }

    /// 직접 입력이 고른 값이 되려면 글이 있어야 한다.
    fn is_row_picked(&self, row: usize) -> bool {
        self.picked[row] && (!self.is_other_row(row) || !self.other.trim().is_empty())
    }

    // cost: time O(o), heap O(o), stack O(1), alloc o
    // vars: o = 선택지 수
    // basis: estimate
    pub(crate) fn picked_labels(&self) -> Vec<String> {
        (0..self.rows())
            .filter(|row| self.is_row_picked(*row))
            .map(|row| {
                if self.is_other_row(row) {
                    self.other.trim().to_owned()
                } else {
                    self.options[row].label.clone()
                }
            })
            .collect()
    }

    // cost: time O(o), heap O(o), stack O(1), alloc o
    // vars: o = 선택지 수
    // basis: estimate
    fn picked_values(&self) -> Vec<String> {
        (0..self.rows())
            .filter(|row| self.is_row_picked(*row))
            .map(|row| {
                if self.is_other_row(row) {
                    self.other.trim().to_owned()
                } else {
                    self.options[row].value.clone()
                }
            })
            .collect()
    }

    fn push_other(&mut self, c: char) {
        self.other.push(c);
        let row = self.cursor;
        self.pick(row);
    }

    fn pop_other(&mut self) {
        self.other.pop();
        if self.other.is_empty() {
            let row = self.cursor;
            self.picked[row] = false;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FieldState {
    Text(String),
    Number { text: String, is_integer: bool },
    Boolean(Option<bool>),
    Choice(ChoiceState),
}

impl FieldState {
    // cost: time O(o), heap O(o), stack O(1), alloc 2
    // vars: o = 선택지 수
    // basis: estimate
    fn new(kind: &InputFieldKind) -> Self {
        match kind {
            InputFieldKind::Text => Self::Text(String::new()),
            InputFieldKind::Integer => Self::Number {
                text: String::new(),
                is_integer: true,
            },
            InputFieldKind::Number => Self::Number {
                text: String::new(),
                is_integer: false,
            },
            InputFieldKind::Boolean => Self::Boolean(None),
            InputFieldKind::Choice {
                options,
                allows_other,
            } => Self::Choice(ChoiceState::new(options, false, *allows_other)),
            InputFieldKind::MultiChoice {
                options,
                allows_other,
            } => Self::Choice(ChoiceState::new(options, true, *allows_other)),
        }
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    fn push(&mut self, c: char) {
        match self {
            Self::Text(text) => text.push(c),
            Self::Number { text, .. } if "+-.eE".contains(c) || c.is_ascii_digit() => text.push(c),
            Self::Choice(choice) if choice.is_other_row(choice.cursor) => choice.push_other(c),
            _ => {}
        }
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    fn pop(&mut self) {
        match self {
            Self::Text(text) | Self::Number { text, .. } => {
                text.pop();
            }
            Self::Choice(choice) if choice.is_other_row(choice.cursor) => choice.pop_other(),
            _ => {}
        }
    }

    // cost: time O(o), heap O(o), stack O(1), alloc 1
    // vars: o = 선택지 수
    // basis: estimate
    /// 비운 칸은 `Ok(None)`.
    fn value(&self) -> Result<Option<InputValue>, FormError> {
        match self {
            Self::Text(text) => {
                Ok((!text.trim().is_empty()).then(|| InputValue::Text(text.clone())))
            }
            Self::Number { text, is_integer } => {
                let text = text.trim();
                if text.is_empty() {
                    return Ok(None);
                }
                if *is_integer {
                    return text
                        .parse::<i64>()
                        .map(|number| Some(InputValue::Integer(number)))
                        .map_err(|_| FormError::NotInteger);
                }
                text.parse::<f64>()
                    .ok()
                    .filter(|number| number.is_finite())
                    .map(|number| Some(InputValue::Number(number)))
                    .ok_or(FormError::NotNumber)
            }
            Self::Boolean(flag) => Ok(flag.map(InputValue::Boolean)),
            Self::Choice(choice) => {
                let values = choice.picked_values();
                Ok((!values.is_empty()).then_some(InputValue::Selected(values)))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct InputForm {
    pub(crate) request: InputRequest,
    pub(crate) states: Vec<FieldState>,
    pub(crate) focus: usize,
    /// 제출을 막은 칸과 이유. 다음 키에서 지운다.
    pub(crate) error: Option<(usize, FormError)>,
}

impl InputForm {
    // cost: time O(f·o), heap O(f·o), stack O(1), alloc f
    // vars: f = 칸 수, o = 선택지 수
    // basis: estimate
    pub(crate) fn new(request: InputRequest) -> Self {
        let states = request
            .fields
            .iter()
            .map(|field| FieldState::new(&field.kind))
            .collect();
        Self {
            request,
            states,
            focus: 0,
            error: None,
        }
    }

    pub(crate) fn is_link(&self) -> bool {
        self.request.url.is_some()
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    /// 붙여넣기는 글 칸과 직접 입력 줄에만 들어가고 줄바꿈 같은 제어 문자는 버린다.
    pub(crate) fn paste(&mut self, text: &str) {
        self.error = None;
        let Some(state) = self.states.get_mut(self.focus) else {
            return;
        };
        text.chars()
            .filter(|c| !c.is_control())
            .for_each(|c| state.push(c));
    }

    // cost: time O(f·o), heap O(f·o), stack O(1), alloc f
    // vars: f = 칸 수, o = 선택지 수
    // basis: estimate
    /// 키 하나를 처리한다. `Ctrl+D`는 거절, `Esc`는 취소이고 칸 입력은 칸 종류를 따른다.
    pub(crate) fn on_key(&mut self, key: KeyEvent) -> FormEvent {
        self.error = None;
        let plain = !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        match key.code {
            KeyCode::Esc => FormEvent::Answer(InputAnswer::Cancel),
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                FormEvent::Answer(InputAnswer::Decline)
            }
            KeyCode::Char('d' | 'D') if plain && self.is_link() => {
                FormEvent::Answer(InputAnswer::Decline)
            }
            KeyCode::Enter => self.on_enter(),
            KeyCode::Tab => self.move_focus(1),
            KeyCode::BackTab => self.move_focus(-1),
            KeyCode::Up => self.on_vertical(-1),
            KeyCode::Down => self.on_vertical(1),
            KeyCode::Backspace => self.edit(FieldState::pop),
            KeyCode::Char(c) if plain => self.on_char(c),
            _ => FormEvent::Edited,
        }
    }

    fn edit(&mut self, change: impl FnOnce(&mut FieldState)) -> FormEvent {
        if let Some(state) = self.states.get_mut(self.focus) {
            change(state);
        }
        FormEvent::Edited
    }

    fn on_char(&mut self, c: char) -> FormEvent {
        let Some(state) = self.states.get_mut(self.focus) else {
            return FormEvent::Edited;
        };
        match state {
            FieldState::Boolean(flag) => match c {
                ' ' => *flag = Some(!flag.unwrap_or(false)),
                'y' | 'Y' => *flag = Some(true),
                'n' | 'N' => *flag = Some(false),
                _ => {}
            },
            FieldState::Choice(choice) if c == ' ' && !choice.is_other_row(choice.cursor) => {
                let row = choice.cursor;
                choice.toggle(row);
            }
            other => other.push(c),
        }
        FormEvent::Edited
    }

    /// 단일 선택은 줄을 고른 뒤, 마지막 칸이면 제출하고 아니면 다음 칸으로 간다.
    fn on_enter(&mut self) -> FormEvent {
        if self.is_link() {
            return FormEvent::Answer(InputAnswer::Submit { values: Vec::new() });
        }
        if let Some(FieldState::Choice(choice)) = self.states.get_mut(self.focus)
            && !choice.is_multi
        {
            let row = choice.cursor;
            if !choice.is_other_row(row) {
                choice.pick(row);
            }
        }
        if self.focus + 1 < self.states.len() {
            return self.move_focus(1);
        }
        self.submit()
    }

    fn move_focus(&mut self, step: isize) -> FormEvent {
        let last = self.states.len().saturating_sub(1);
        self.focus = self.focus.saturating_add_signed(step).min(last);
        FormEvent::Edited
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    /// 선택 칸 안에서는 줄을 옮기고, 끝에서 더 가면 이웃 칸으로 간다.
    fn on_vertical(&mut self, step: isize) -> FormEvent {
        if let Some(FieldState::Choice(choice)) = self.states.get_mut(self.focus) {
            let next = choice.cursor.checked_add_signed(step);
            if let Some(next) = next.filter(|next| *next < choice.rows()) {
                choice.cursor = next;
                return FormEvent::Edited;
            }
        }
        let event = self.move_focus(step);
        if let Some(FieldState::Choice(choice)) = self.states.get_mut(self.focus) {
            choice.cursor = if step < 0 {
                choice.rows().saturating_sub(1)
            } else {
                0
            };
        }
        event
    }

    // cost: time O(f·o), heap O(f·o), stack O(1), alloc f
    // vars: f = 칸 수, o = 선택지 수
    // basis: estimate
    /// 필수 칸이 비었거나 숫자를 읽지 못하면 그 칸으로 가서 이유를 보이고 제출하지 않는다. 비워 둔 선택 칸은
    /// 답에 싣지 않는다.
    pub(crate) fn submit(&mut self) -> FormEvent {
        let mut values = Vec::new();
        for (index, field) in self.request.fields.iter().enumerate() {
            match self.states[index].value() {
                Ok(Some(value)) => values.push((field.id.clone(), value)),
                Ok(None) if field.is_required => return self.reject(index, FormError::Required),
                Ok(None) => {}
                Err(error) => return self.reject(index, error),
            }
        }
        FormEvent::Answer(InputAnswer::Submit { values })
    }

    fn reject(&mut self, index: usize, error: FormError) -> FormEvent {
        self.focus = index;
        self.error = Some((index, error));
        FormEvent::Edited
    }

    pub(crate) fn field(&self, index: usize) -> Option<&InputField> {
        self.request.fields.get(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn option(value: &str) -> InputOption {
        InputOption {
            value: value.to_owned(),
            label: value.to_uppercase(),
            description: String::new(),
        }
    }

    fn field(id: &str, kind: InputFieldKind, is_required: bool) -> InputField {
        InputField {
            id: id.to_owned(),
            title: id.to_owned(),
            description: String::new(),
            kind,
            is_required,
            is_secret: false,
        }
    }

    fn form(fields: Vec<InputField>) -> InputForm {
        InputForm::new(InputRequest {
            message: String::new(),
            fields,
            url: None,
        })
    }

    fn press(form: &mut InputForm, code: KeyCode) -> FormEvent {
        form.on_key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn type_text(form: &mut InputForm, text: &str) {
        text.chars().for_each(|c| {
            press(form, KeyCode::Char(c));
        });
    }

    fn all_kinds() -> InputForm {
        form(vec![
            field("title", InputFieldKind::Text, true),
            field("count", InputFieldKind::Integer, true),
            field("enabled", InputFieldKind::Boolean, true),
            field(
                "choice",
                InputFieldKind::Choice {
                    options: vec![option("a"), option("b")],
                    allows_other: false,
                },
                true,
            ),
            field(
                "tags",
                InputFieldKind::MultiChoice {
                    options: vec![option("x"), option("y")],
                    allows_other: false,
                },
                true,
            ),
        ])
    }

    #[test]
    fn elicitation_form_collects_every_field_kind_into_one_answer() {
        let mut form = all_kinds();

        type_text(&mut form, "Saturn");
        press(&mut form, KeyCode::Enter);
        type_text(&mut form, "3");
        press(&mut form, KeyCode::Enter);
        press(&mut form, KeyCode::Char(' '));
        press(&mut form, KeyCode::Enter);
        press(&mut form, KeyCode::Down);
        press(&mut form, KeyCode::Enter);
        press(&mut form, KeyCode::Char(' '));
        press(&mut form, KeyCode::Down);
        press(&mut form, KeyCode::Char(' '));
        let event = press(&mut form, KeyCode::Enter);

        assert_eq!(
            event,
            FormEvent::Answer(InputAnswer::Submit {
                values: vec![
                    ("title".to_owned(), InputValue::Text("Saturn".to_owned())),
                    ("count".to_owned(), InputValue::Integer(3)),
                    ("enabled".to_owned(), InputValue::Boolean(true)),
                    (
                        "choice".to_owned(),
                        InputValue::Selected(vec!["b".to_owned()])
                    ),
                    (
                        "tags".to_owned(),
                        InputValue::Selected(vec!["x".to_owned(), "y".to_owned()])
                    ),
                ],
            })
        );
    }

    #[test]
    fn elicitation_boolean_takes_space_and_yes_no_keys() {
        let mut form = form(vec![field("on", InputFieldKind::Boolean, true)]);

        press(&mut form, KeyCode::Char(' '));
        assert_eq!(form.states[0], FieldState::Boolean(Some(true)));
        press(&mut form, KeyCode::Char(' '));
        assert_eq!(form.states[0], FieldState::Boolean(Some(false)));
        press(&mut form, KeyCode::Char('y'));
        assert_eq!(form.states[0], FieldState::Boolean(Some(true)));
        press(&mut form, KeyCode::Char('n'));
        assert_eq!(form.states[0], FieldState::Boolean(Some(false)));
    }

    #[test]
    fn elicitation_missing_required_field_blocks_the_answer_and_takes_focus() {
        let mut form = all_kinds();
        form.focus = 4;

        let event = press(&mut form, KeyCode::Enter);

        assert_eq!(event, FormEvent::Edited);
        assert_eq!(form.focus, 0);
        assert_eq!(form.error, Some((0, FormError::Required)));
        press(&mut form, KeyCode::Char('x'));
        assert_eq!(form.error, None);
    }

    #[test]
    fn elicitation_optional_field_left_empty_is_not_sent() {
        let mut form = form(vec![
            field("name", InputFieldKind::Text, true),
            field("note", InputFieldKind::Text, false),
        ]);

        type_text(&mut form, "a");
        press(&mut form, KeyCode::Enter);
        let event = press(&mut form, KeyCode::Enter);

        assert_eq!(
            event,
            FormEvent::Answer(InputAnswer::Submit {
                values: vec![("name".to_owned(), InputValue::Text("a".to_owned()))],
            })
        );
    }

    #[test]
    fn elicitation_number_fields_reject_unreadable_values() {
        let mut integer = form(vec![field("n", InputFieldKind::Integer, true)]);
        let mut number = form(vec![field("n", InputFieldKind::Number, true)]);

        type_text(&mut integer, "1.5abc");
        press(&mut integer, KeyCode::Enter);
        type_text(&mut number, "1e");
        press(&mut number, KeyCode::Enter);

        assert_eq!(integer.states[0].value(), Err(FormError::NotInteger));
        assert_eq!(integer.error, Some((0, FormError::NotInteger)));
        assert_eq!(number.error, Some((0, FormError::NotNumber)));
        press(&mut number, KeyCode::Backspace);
        press(&mut number, KeyCode::Backspace);
        type_text(&mut number, "-2.5");
        assert_eq!(
            press(&mut number, KeyCode::Enter),
            FormEvent::Answer(InputAnswer::Submit {
                values: vec![("n".to_owned(), InputValue::Number(-2.5))],
            })
        );
    }

    #[test]
    fn elicitation_other_row_takes_typed_text_and_clears_the_single_pick() {
        let mut form = form(vec![field(
            "q",
            InputFieldKind::Choice {
                options: vec![option("a")],
                allows_other: true,
            },
            true,
        )]);

        press(&mut form, KeyCode::Enter);
        press(&mut form, KeyCode::Down);
        type_text(&mut form, "my own ");
        let event = press(&mut form, KeyCode::Enter);

        assert_eq!(
            event,
            FormEvent::Answer(InputAnswer::Submit {
                values: vec![(
                    "q".to_owned(),
                    InputValue::Selected(vec!["my own".to_owned()])
                )],
            })
        );
    }

    #[test]
    fn elicitation_up_and_down_cross_between_fields_at_the_edges() {
        let mut form = all_kinds();

        press(&mut form, KeyCode::Down);
        assert_eq!(form.focus, 1);
        press(&mut form, KeyCode::Tab);
        press(&mut form, KeyCode::Tab);
        assert_eq!(form.focus, 3);
        press(&mut form, KeyCode::Down);
        press(&mut form, KeyCode::Down);
        assert_eq!(form.focus, 4);
        press(&mut form, KeyCode::Up);
        assert_eq!(form.focus, 3);
        press(&mut form, KeyCode::BackTab);
        assert_eq!(form.focus, 2);
    }

    #[test]
    fn elicitation_decline_and_cancel_keys_end_the_request() {
        let mut declined = all_kinds();
        let mut cancelled = all_kinds();

        let decline = declined.on_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
        let cancel = press(&mut cancelled, KeyCode::Esc);

        assert_eq!(decline, FormEvent::Answer(InputAnswer::Decline));
        assert_eq!(cancel, FormEvent::Answer(InputAnswer::Cancel));
    }

    #[test]
    fn elicitation_link_request_accepts_declines_and_cancels() {
        let link = || {
            InputForm::new(InputRequest {
                message: "open it".to_owned(),
                fields: Vec::new(),
                url: Some("https://example.invalid".to_owned()),
            })
        };

        assert_eq!(
            press(&mut link(), KeyCode::Enter),
            FormEvent::Answer(InputAnswer::Submit { values: Vec::new() })
        );
        assert_eq!(
            press(&mut link(), KeyCode::Char('d')),
            FormEvent::Answer(InputAnswer::Decline)
        );
        assert_eq!(
            press(&mut link(), KeyCode::Esc),
            FormEvent::Answer(InputAnswer::Cancel)
        );
    }

    #[test]
    fn elicitation_paste_goes_to_text_fields_without_control_characters() {
        let mut form = form(vec![field("t", InputFieldKind::Text, true)]);

        form.paste("one\ntwo\t");

        assert_eq!(form.states[0], FieldState::Text("onetwo".to_owned()));
    }
}
