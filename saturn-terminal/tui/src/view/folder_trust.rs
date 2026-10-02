//! 폴더 설정 신뢰 창. 처음 보거나 내용이 바뀐 폴더 설정 파일을 적용할지 묻는다.
//! 설계: docs/design/tui.md

use std::path::PathBuf;

use ratatui::Frame;
use ratatui::layout::Rect;

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::i18n::{self, Lang};
use crate::view::{EMPHASIS, SELECTED, render_window};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrustChoice {
    Apply,
    /// 설계 표에 키와 문구가 없어 방향키와 `Enter`로만 고른다.
    Second,
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FolderTrust {
    pub path: PathBuf,
    pub fingerprint: String,
    pub applied: Vec<String>,
    pub ignored: Vec<String>,
    /// 처음 보면 빈 목록.
    pub changed: Vec<String>,
    pub selected: TrustChoice,
}

impl FolderTrust {
    pub(crate) fn up(&mut self) {
        self.selected = match self.selected {
            TrustChoice::Apply | TrustChoice::Second => TrustChoice::Apply,
            TrustChoice::Quit => TrustChoice::Second,
        };
    }

    pub(crate) fn down(&mut self) {
        self.selected = match self.selected {
            TrustChoice::Apply => TrustChoice::Second,
            TrustChoice::Second | TrustChoice::Quit => TrustChoice::Quit,
        };
    }
}

#[derive(Debug)]
pub(crate) struct FolderTrustView<'a> {
    pub trust: &'a FolderTrust,
    pub lang: Lang,
}

impl FolderTrustView<'_> {
    // cost: time O(m), heap O(m), stack O(1)
    // vars: m = 항목과 바뀐 줄의 글자 수
    // basis: estimate
    pub(crate) fn render(&self, frame: &mut Frame, area: Rect) {
        let lang = self.lang;
        let trust = self.trust;
        let mut lines = vec![
            Line::from(format!(
                "{}: {}",
                lang.tr(i18n::TRUST_PATH),
                trust.path.display()
            )),
            Line::from(format!(
                "{}: {}",
                lang.tr(i18n::TRUST_FINGERPRINT),
                trust.fingerprint
            )),
        ];
        for (title, items) in [
            (i18n::TRUST_APPLIED, &trust.applied),
            (i18n::TRUST_IGNORED, &trust.ignored),
            (i18n::TRUST_CHANGED, &trust.changed),
        ] {
            if items.is_empty() {
                continue;
            }
            lines.push(Line::from(Span::styled(lang.tr(title), EMPHASIS)));
            lines.extend(items.iter().map(|item| Line::from(format!("  {item}"))));
        }
        lines.push(Line::from(""));
        for (choice, text) in [
            (
                TrustChoice::Apply,
                format!("1 {}", lang.tr(i18n::TRUST_APPLY)),
            ),
            (
                TrustChoice::Second,
                format!("  {}", lang.tr(i18n::TRUST_SKIP)),
            ),
            (
                TrustChoice::Quit,
                format!("3 {}", lang.tr(i18n::TRUST_QUIT)),
            ),
        ] {
            let style = if choice == trust.selected {
                SELECTED
            } else {
                Style::new()
            };
            lines.push(Line::from(Span::styled(text, style)));
        }
        render_window(frame, area, lang.tr(i18n::TRUST_TITLE), lines);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn trust() -> FolderTrust {
        FolderTrust {
            path: PathBuf::from("/repo/.saturn/settings.toml"),
            fingerprint: "ab12".to_string(),
            applied: vec!["model".to_string()],
            ignored: vec!["router.url".to_string()],
            changed: Vec::new(),
            selected: TrustChoice::Apply,
        }
    }

    #[test]
    fn up_down_move_between_three_choices() {
        let mut trust = trust();

        trust.down();
        let second = trust.selected;
        trust.down();
        trust.down();
        let last = trust.selected;
        trust.up();

        assert_eq!(second, TrustChoice::Second);
        assert_eq!(last, TrustChoice::Quit);
        assert_eq!(trust.selected, TrustChoice::Second);
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn render_shows_path_items_and_choices() {
        let trust = trust();
        let mut terminal = Terminal::new(TestBackend::new(70, 16)).unwrap();
        let view = FolderTrustView {
            trust: &trust,
            lang: Lang::Ko,
        };

        terminal
            .draw(|frame| view.render(frame, frame.area()))
            .unwrap();

        let content = crate::view::buffer_lines(terminal.backend().buffer()).join("\n");
        assert!(content.contains("/repo/.saturn/settings.toml"));
        assert!(content.contains("router.url"));
        assert!(content.contains("3 종료"));
    }
}
