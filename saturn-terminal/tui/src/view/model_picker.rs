//! 모델 선택 창(`/model`). 방향키로 고르고 `Enter`로 정한다.
//! 설계: docs/design/tui.md

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use saturn_protocol::ids::Provider;
use saturn_protocol::rpc::{ModelChoice, ModelInfo};

use crate::i18n::{self, Lang};
use crate::view::{MUTED, SELECTED, render_window};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModelPicker {
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
            provider,
            models: None,
            selected: 0,
            current,
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
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(lang.tr(i18n::MODEL_HINT), MUTED)));
        render_window(frame, area, lang.tr(i18n::MODEL_TITLE), lines);
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
            info(Provider::Claude, "opus"),
            info(Provider::Claude, "sonnet"),
            info(Provider::Codex, "gpt-x"),
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
            provider: Provider::Claude,
            model: "sonnet".to_owned(),
        };

        let picker = picker_with_models(Some(current.clone()));

        assert_eq!(picker.selected_choice(), Some(&current));
    }

    #[test]
    fn render_marks_the_pinned_model_and_shows_provider_names() {
        let current = ModelChoice {
            provider: Provider::Codex,
            model: "gpt-x".to_owned(),
        };
        let picker = picker_with_models(Some(current));
        let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
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
        assert!(text.contains("Esc cancel"));
    }
}
