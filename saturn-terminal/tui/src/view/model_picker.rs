//! 모델 선택 창(`/model`). 방향키로 고르고 `Enter`로 정한다.
//! 설계: docs/design/tui.md

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use saturn_protocol::ids::Provider;
use saturn_protocol::rpc::{ModelChoice, ModelInfo, ModelMode};

use crate::i18n::{self, Lang};
use crate::view::{MUTED, SELECTED, render_window};

/// 창을 연 목적. 처음 고르기 창은 `Enter`가 기본 모델을 저장하고, `/model` 창은 이 채팅에 고정한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModelPurpose {
    Pin,
    Default,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModelPicker {
    pub purpose: ModelPurpose,
    /// 지금 기본 모델과 선택 방식. 창이 열린 동안 바뀌면 engine 알림으로 갱신한다.
    pub default: Option<ModelChoice>,
    pub mode: ModelMode,
    /// `/model <provider>`로 연 창이면 그 provider.
    pub provider: Option<Provider>,
    /// 목록이 오기 전에는 `None`.
    pub models: Option<Vec<ModelInfo>>,
    pub selected: usize,
    /// 지금 고정한 모델. 목록에서 표시하고, 목록이 오면 그 줄에서 시작한다.
    pub current: Option<ModelChoice>,
}

impl ModelPicker {
    pub(crate) fn new(provider: Option<Provider>, current: Option<ModelChoice>) -> Self {
        Self {
            purpose: ModelPurpose::Pin,
            default: None,
            mode: ModelMode::Manual,
            provider,
            models: None,
            selected: 0,
            current,
        }
    }

    /// 기본 모델을 아직 고르지 않았을 때 여는 창. 모든 provider의 목록을 보인다.
    pub(crate) fn first_default(mode: ModelMode) -> Self {
        Self {
            purpose: ModelPurpose::Default,
            mode,
            ..Self::new(None, None)
        }
    }

    /// 지금 고정한 모델이 목록에 있으면 그 줄을 고른 채로 둔다.
    pub(crate) fn load(&mut self, models: Vec<ModelInfo>) {
        self.selected = self
            .current
            .as_ref()
            .and_then(|current| models.iter().position(|info| info.choice == *current))
            .unwrap_or(0);
        self.models = Some(models);
    }

    pub(crate) fn up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub(crate) fn down(&mut self) {
        let last = self
            .models
            .as_ref()
            .map_or(0, |models| models.len().saturating_sub(1));
        self.selected = (self.selected + 1).min(last);
    }

    pub(crate) fn selected_choice(&self) -> Option<&ModelChoice> {
        self.models
            .as_ref()
            .and_then(|models| models.get(self.selected))
            .map(|info| &info.choice)
    }
}

#[derive(Debug)]
pub(crate) struct ModelPickerView<'a> {
    pub picker: &'a ModelPicker,
    pub lang: Lang,
}

impl ModelPickerView<'_> {
    // cost: time O(m), heap O(m), stack O(1)
    // vars: m = 모델 수
    // basis: estimate
    pub(crate) fn render(&self, frame: &mut Frame, area: Rect) {
        let lang = self.lang;
        let picker = self.picker;
        let mut lines: Vec<Line<'static>> = match &picker.models {
            None => vec![Line::from(Span::styled(
                lang.tr(i18n::MODEL_LOADING),
                MUTED,
            ))],
            Some(models) if models.is_empty() => {
                vec![Line::from(Span::styled(lang.tr(i18n::MODEL_EMPTY), MUTED))]
            }
            Some(models) => models
                .iter()
                .enumerate()
                .map(|(index, info)| self.row(index, info))
                .collect(),
        };
        let (title, hint, head) = match picker.purpose {
            ModelPurpose::Default => (
                i18n::MODEL_DEFAULT_TITLE,
                i18n::MODEL_DEFAULT_HINT,
                vec![
                    Line::from(Span::styled(lang.tr(i18n::MODEL_DEFAULT_INTRO), MUTED)),
                    Line::from(""),
                ],
            ),
            ModelPurpose::Pin => (
                i18n::MODEL_TITLE,
                i18n::MODEL_HINT,
                vec![
                    Line::from(Span::styled(self.status(), MUTED)),
                    Line::from(""),
                ],
            ),
        };
        lines.splice(0..0, head);
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(lang.tr(hint), MUTED)));
        render_window(frame, area, lang.tr(title), lines);
    }

    /// 지금 기본 모델과 선택 방식 한 줄.
    fn status(&self) -> String {
        let lang = self.lang;
        let model = self.picker.default.as_ref().map_or_else(
            || lang.tr(i18n::MODEL_NONE).to_owned(),
            |choice| {
                format!(
                    "{} · {}",
                    i18n::provider_name(choice.provider),
                    choice.model
                )
            },
        );
        lang.tr(i18n::MODEL_STATUS)
            .replace("{model}", &model)
            .replace("{mode}", lang.tr(i18n::model_mode_name(self.picker.mode)))
    }

    fn row(&self, index: usize, info: &ModelInfo) -> Line<'static> {
        let marker = if self.picker.current.as_ref() == Some(&info.choice) {
            "* "
        } else {
            "  "
        };
        let text = format!(
            "{marker}{} · {}",
            i18n::provider_name(info.choice.provider),
            info.name
        );
        let style = if index == self.picker.selected {
            SELECTED
        } else {
            Style::new()
        };
        Line::from(Span::styled(text, style))
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::view::buffer_lines;

    fn info(provider: Provider, model: &str) -> ModelInfo {
        ModelInfo {
            choice: ModelChoice {
                provider,
                model: model.to_owned(),
            },
            name: model.to_owned(),
        }
    }

    fn picker_with_models(current: Option<ModelChoice>) -> ModelPicker {
        let mut picker = ModelPicker::new(None, current);
        picker.load(vec![
            info(Provider::from_static("claude"), "opus"),
            info(Provider::from_static("claude"), "sonnet"),
            info(Provider::from_static("codex"), "gpt-x"),
        ]);
        picker
    }

    #[test]
    fn selection_stays_inside_the_list() {
        let mut picker = picker_with_models(None);

        picker.up();
        let first = picker.selected_choice().cloned();
        (0..5).for_each(|_| picker.down());
        let last = picker.selected_choice().cloned();

        assert_eq!(first.map(|choice| choice.model), Some("opus".to_owned()));
        assert_eq!(last.map(|choice| choice.model), Some("gpt-x".to_owned()));
    }

    #[test]
    fn list_starts_on_the_pinned_model() {
        let current = ModelChoice {
            provider: Provider::from_static("claude"),
            model: "sonnet".to_owned(),
        };

        let picker = picker_with_models(Some(current.clone()));

        assert_eq!(picker.selected_choice(), Some(&current));
    }

    #[test]
    fn render_marks_the_pinned_model_and_shows_provider_names() {
        let current = ModelChoice {
            provider: Provider::from_static("codex"),
            model: "gpt-x".to_owned(),
        };
        let picker = picker_with_models(Some(current));
        let mut terminal = Terminal::new(TestBackend::new(100, 14)).unwrap();
        let view = ModelPickerView {
            picker: &picker,
            lang: Lang::En,
        };

        terminal
            .draw(|frame| view.render(frame, frame.area()))
            .unwrap();

        let text = buffer_lines(terminal.backend().buffer()).join("\n");
        assert!(text.contains("  claude · opus"));
        assert!(text.contains("* codex · gpt-x"));
        assert!(text.contains("Esc close"));
    }

    fn render_text(picker: &ModelPicker) -> String {
        let mut terminal = Terminal::new(TestBackend::new(100, 14)).unwrap();
        let view = ModelPickerView {
            picker,
            lang: Lang::En,
        };
        terminal
            .draw(|frame| view.render(frame, frame.area()))
            .unwrap();
        buffer_lines(terminal.backend().buffer()).join("\n")
    }

    #[test]
    fn first_choice_window_asks_for_a_default_model_and_lists_every_model() {
        let mut picker = ModelPicker::first_default(ModelMode::Auto);
        picker.load(vec![
            info(Provider::from_static("claude"), "opus"),
            info(Provider::from_static("codex"), "gpt-x"),
        ]);

        let text = render_text(&picker);

        assert!(text.contains("Choose a default model"));
        assert!(text.contains("claude · opus"));
        assert!(text.contains("codex · gpt-x"));
        assert!(text.contains("Enter save as default"));
    }

    #[test]
    fn model_window_shows_the_default_model_and_the_selection_mode() {
        let mut picker = picker_with_models(None);
        picker.default = Some(ModelChoice {
            provider: Provider::from_static("claude"),
            model: "opus".to_owned(),
        });
        picker.mode = ModelMode::Manual;

        let text = render_text(&picker);

        assert!(text.contains("Default model claude · opus · selection Manual"));
    }

    #[test]
    fn model_window_without_a_default_shows_none() {
        let picker = picker_with_models(None);

        assert!(render_text(&picker).contains("Default model none · selection Manual"));
    }
}
