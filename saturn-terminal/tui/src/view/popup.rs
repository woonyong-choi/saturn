//! 팝업: `/` 명령 목록과 값 목록, `@` 파일 목록, `$` 스킬 목록.
//! 설계: docs/design/tui.md

use ratatui::Frame;
use ratatui::layout::Rect;

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::view::{SELECTED, text_width, truncate};

pub(crate) const MAX_ROWS: usize = 8;
/// 큰 저장소에서 화면이 멈추지 않게 둔 상한(초안).
pub(crate) const MAX_FILES: usize = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PopupKind {
    Command,
    Value,
    File,
    Skill,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PopupItem {
    pub value: String,
    pub description: String,
    /// 파일 목록은 비운다.
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Popup {
    pub kind: PopupKind,
    /// `/ju`의 `ju`.
    pub token: String,
    pub items: Vec<PopupItem>,
    pub selected: usize,
}

impl Popup {
    pub(crate) fn open(kind: PopupKind, candidates: Vec<PopupItem>) -> Self {
        Self {
            kind,
            token: String::new(),
            items: candidates,
            selected: 0,
        }
    }

    // cost: time O(p·q), heap O(p), stack O(1)
    // vars: p = 후보 수, q = token.len()
    // basis: estimate
    /// 앞부분 일치 우선, 그다음 포함.
    pub(crate) fn filter(&mut self, token: &str, candidates: &[PopupItem]) {
        self.token = token.to_string();
        let prefixed = candidates.iter().filter(|c| c.value.starts_with(token));
        let contained = candidates
            .iter()
            .filter(|c| !c.value.starts_with(token) && c.value.contains(token));
        self.items = prefixed.chain(contained).cloned().collect();
        self.selected = 0;
    }

    /// 첫 행에서 멈춘다.
    pub(crate) fn up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// 끝 행에서 멈춘다.
    pub(crate) fn down(&mut self) {
        if self.selected + 1 < self.items.len() {
            self.selected += 1;
        }
    }

    pub(crate) fn selected(&self) -> Option<&PopupItem> {
        self.items.get(self.selected)
    }

    pub(crate) fn height(&self) -> u16 {
        self.items.len().min(MAX_ROWS) as u16
    }
}

/// 같은 토큰이면 다시 띄우지 않는다.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct PopupSuppress {
    token: Option<String>,
}

impl PopupSuppress {
    pub(crate) fn suppress(&mut self, token: &str) {
        self.token = Some(token.to_string());
    }

    /// 토큰이 바뀌면 억제를 풀고 `true`.
    pub(crate) fn allows(&mut self, token: &str) -> bool {
        match &self.token {
            Some(suppressed) if suppressed == token => false,
            Some(_) => {
                self.token = None;
                true
            }
            None => true,
        }
    }
}

// cost: time O(f log f), heap O(f), stack O(1), io d
// vars: f = 훑은 항목 수(최대 MAX_FILES), d = 읽은 폴더 수
// basis: estimate
/// `.git`과 `.gitignore`의 글로브 없는 이름을 빼고 `MAX_FILES`개까지 모은다(초안).
pub(crate) fn file_candidates(workdir: &std::path::Path) -> Vec<PopupItem> {
    let ignored = ignore_names(workdir);
    let mut found = Vec::new();
    let mut pending = vec![workdir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            if found.len() >= MAX_FILES {
                return found;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == ".git" || ignored.contains(&name) {
                continue;
            }
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            if let Ok(relative) = path.strip_prefix(workdir) {
                found.push(PopupItem {
                    value: format!("@{}", relative.display()),
                    description: String::new(),
                    source: String::new(),
                });
            }
        }
    }
    found
}

#[derive(Debug)]
pub(crate) struct PopupView<'a> {
    pub popup: &'a Popup,
}

impl PopupView<'_> {
    // cost: time O(r·w), heap O(r·w), stack O(1)
    // vars: r = 행 수(최대 8), w = 칸 폭
    // basis: estimate
    pub(crate) fn render(&self, frame: &mut Frame, area: Rect) {
        let width = usize::from(area.width);
        let start = (self.popup.selected + 1).saturating_sub(MAX_ROWS);
        let rows: Vec<Line> = self
            .popup
            .items
            .iter()
            .enumerate()
            .skip(start)
            .take(usize::from(area.height))
            .map(|(index, item)| {
                let selected = index == self.popup.selected;
                let text = row_text(item, selected, width);
                let style = if selected { SELECTED } else { Style::new() };
                Line::from(Span::styled(text, style))
            })
            .collect();
        frame.render_widget(Paragraph::new(rows), area);
    }
}

// cost: time O(n), heap O(n), stack O(1), io 1
// vars: n = .gitignore 길이
// basis: estimate
/// `.gitignore`에서 글로브 없는 이름(`target`, `/node_modules/`)만 읽는다.
fn ignore_names(workdir: &std::path::Path) -> Vec<String> {
    let Ok(content) = std::fs::read_to_string(workdir.join(".gitignore")) else {
        return Vec::new();
    };
    content
        .lines()
        .map(|line| line.trim().trim_matches('/'))
        .filter(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with('!'))
        .filter(|line| !line.contains(['*', '?', '[', '/']))
        .map(str::to_string)
        .collect()
}

fn row_text(item: &PopupItem, selected: bool, width: usize) -> String {
    let marker = if selected { "› " } else { "  " };
    let source_width = text_width(&item.source);
    let room = width.saturating_sub(source_width + 3);
    let left = if item.description.is_empty() {
        format!("{marker}{}", item.value)
    } else {
        format!("{marker}{}  {}", item.value, item.description)
    };
    let left = truncate(&left, room);
    let gap = width.saturating_sub(text_width(&left) + source_width);
    format!("{left}{}{}", " ".repeat(gap), item.source)
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn item(value: &str) -> PopupItem {
        PopupItem {
            value: value.to_string(),
            description: String::new(),
            source: "Saturn".to_string(),
        }
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    fn candidates() -> Vec<PopupItem> {
        ["help", "send", "model set", "usage"]
            .into_iter()
            .map(item)
            .collect()
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn filter_prefers_prefix_then_contains() {
        let mut popup = Popup::open(PopupKind::Command, candidates());

        popup.filter("s", &candidates());

        let values: Vec<&str> = popup.items.iter().map(|i| i.value.as_str()).collect();
        assert_eq!(values, vec!["send", "model set", "usage"]);
        assert_eq!(popup.selected, 0);
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn up_down_stop_at_edges() {
        let mut popup = Popup::open(PopupKind::Command, candidates());

        popup.up();
        let top = popup.selected;
        (0..10).for_each(|_| popup.down());

        assert_eq!(top, 0);
        assert_eq!(popup.selected().map(|i| i.value.as_str()), Some("usage"));
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn height_caps_at_max_rows() {
        let many: Vec<PopupItem> = (0..20).map(|i| item(&format!("c{i}"))).collect();

        assert_eq!(Popup::open(PopupKind::Command, many).height(), 8);
    }

    #[test]
    fn suppress_blocks_same_token_until_it_changes() {
        let mut suppress = PopupSuppress::default();
        suppress.suppress("/ju");

        assert!(!suppress.allows("/ju"));
        assert!(suppress.allows("/jud"));
        assert!(suppress.allows("/ju"));
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn file_candidates_skip_git_and_ignored_names() {
        let root = tempfile::tempdir().unwrap();
        let write = |relative: &str| {
            let path = root.path().join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "x").unwrap();
        };
        write("src/lib.rs");
        write(".git/config");
        write("target/debug/out.bin");
        write("src/target/inner.rs");
        write("notes.log");
        std::fs::write(root.path().join(".gitignore"), "target\nnotes.log\n").unwrap();

        let mut files: Vec<String> = file_candidates(root.path())
            .into_iter()
            .map(|f| f.value)
            .collect();
        files.sort();

        assert_eq!(files, ["@.gitignore", "@src/lib.rs"]);
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn render_marks_selected_row_and_source() {
        let popup = Popup::open(PopupKind::Command, candidates());
        let mut terminal = Terminal::new(TestBackend::new(30, 2)).unwrap();
        let view = PopupView { popup: &popup };

        terminal
            .draw(|frame| view.render(frame, frame.area()))
            .unwrap();

        let buffer = terminal.backend().buffer();
        let row: String = (0..30).map(|x| buffer[(x, 0)].symbol()).collect();
        assert!(row.starts_with("› help"));
        assert!(row.ends_with("Saturn"));
    }
}
