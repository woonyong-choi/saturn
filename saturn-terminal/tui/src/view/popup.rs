//! 팝업: `/` 명령 목록과 값 목록, `@` 파일 목록, `$` 스킬 목록.
//!
//! 설계: docs/design/tui.md(영역 팝업, 키 팝업).
//! - 글자 입력마다 입력 토큰으로 목록을 거른다. 명령 목록은 최대 8행, 오른쪽에 출처(`Saturn` 또는 provider)를 보인다.
//! - `Enter`: 명령 목록이면 고른 명령의 전체 경로를 입력창에 기입, 값 목록이면 값 선택. `Tab`: 전체 경로까지 완성.
//! - `Esc`: 해제하고 입력 토큰이 바뀔 때까지 다시 띄우지 않는다.

use ratatui::Frame;
use ratatui::layout::Rect;

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::i18n::Lang;
use crate::view::{SELECTED, text_width, truncate};

/// 팝업 최대 행 수.
pub const MAX_ROWS: usize = 8;
/// `@` 파일 목록 후보 상한. 큰 저장소에서 화면이 멈추지 않게 한다. 초안 값이다(docs/design/tui.md 초안 값).
pub const MAX_FILES: usize = 2_000;

/// 팝업 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PopupKind {
    /// `/` 명령 목록(`commands::SATURN_COMMANDS`와 provider 명령).
    Command,
    /// 명령을 고른 뒤 그 명령의 값 목록(`/usage` → `chat`, `today`, `week`, `all`).
    Value,
    /// `@` 작업 폴더 파일 목록.
    File,
    /// `$` 메인 에이전트 provider의 스킬 목록. 다른 provider는 `$공급자 이름`. 목록은 `Notification::Commands`의 `is_skill` 항목.
    Skill,
}

/// 목록 한 행.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PopupItem {
    /// 기입할 값(명령 전체 경로, 파일 경로, 스킬 이름).
    pub value: String,
    /// 설명.
    pub description: String,
    /// 오른쪽 출처 표시(`Saturn`, `codex`, `claude`). 파일 목록은 비운다.
    pub source: String,
}

/// 떠 있는 팝업.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Popup {
    /// 종류.
    pub kind: PopupKind,
    /// 거르는 입력 토큰(`/ju`의 `ju`).
    pub token: String,
    /// 거른 목록.
    pub items: Vec<PopupItem>,
    /// 강조 행.
    pub selected: usize,
}

impl Popup {
    /// 종류와 전체 후보로 연다. 토큰은 빈 문자열.
    pub fn open(kind: PopupKind, candidates: Vec<PopupItem>) -> Self {
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
    /// 토큰이 바뀌면 다시 거른다. 앞부분 일치 우선, 그다음 포함. 강조는 첫 행으로.
    pub fn filter(&mut self, token: &str, candidates: &[PopupItem]) {
        self.token = token.to_string();
        let prefixed = candidates.iter().filter(|c| c.value.starts_with(token));
        let contained = candidates
            .iter()
            .filter(|c| !c.value.starts_with(token) && c.value.contains(token));
        self.items = prefixed.chain(contained).cloned().collect();
        self.selected = 0;
    }

    /// `↑` 위로. 첫 행에서 멈춘다.
    pub fn up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// `↓` 아래로. 끝 행에서 멈춘다.
    pub fn down(&mut self) {
        if self.selected + 1 < self.items.len() {
            self.selected += 1;
        }
    }

    /// 강조한 행.
    pub fn selected(&self) -> Option<&PopupItem> {
        self.items.get(self.selected)
    }

    /// 그릴 행 수(최대 8).
    pub fn height(&self) -> u16 {
        self.items.len().min(MAX_ROWS) as u16
    }
}

/// `Esc`로 닫은 뒤의 억제. 같은 토큰이면 다시 띄우지 않는다.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PopupSuppress {
    token: Option<String>,
}

impl PopupSuppress {
    /// 이 토큰에서 닫았다고 기록한다.
    pub fn suppress(&mut self, token: &str) {
        self.token = Some(token.to_string());
    }

    /// 지금 토큰에서 띄워도 되는지. 토큰이 바뀌면 억제를 풀고 참.
    pub fn allows(&mut self, token: &str) -> bool {
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
/// 작업 폴더 파일 후보. `.git` 폴더와 무시 파일은 뺀다. 경로는 작업 폴더 기준 상대 경로이고 값은 `@경로`.
/// 무시 파일은 작업 폴더 `.gitignore`의 글로브 없는 이름만 본다(초안). 후보는 `MAX_FILES`개까지.
pub fn file_candidates(workdir: &std::path::Path) -> Vec<PopupItem> {
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

/// 팝업 그리기.
#[derive(Debug)]
pub struct PopupView<'a> {
    /// 그릴 팝업.
    pub popup: &'a Popup,
    /// 화면 언어.
    pub lang: Lang,
}

impl PopupView<'_> {
    // cost: time O(r·w), heap O(r·w), stack O(1)
    // vars: r = 행 수(최대 8), w = 칸 폭
    // basis: estimate
    /// `› 값  설명 ... 출처` 행을 최대 8행 그린다. 강조 행은 `›`, 출처는 오른쪽 정렬.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
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

/// 팝업 한 행 `› 값  설명 ... 출처`. 출처는 오른쪽 끝에 붙인다.
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
        ["help", "send", "judge version", "usage"]
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
        assert_eq!(values, vec!["send", "judge version", "usage"]);
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
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));

        let files = file_candidates(root);

        assert!(files.iter().any(|f| f.value == "@src/lib.rs"));
        assert!(files.iter().all(|f| !f.value.starts_with("@.git/")));
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn render_marks_selected_row_and_source() {
        let popup = Popup::open(PopupKind::Command, candidates());
        let mut terminal = Terminal::new(TestBackend::new(30, 2)).unwrap();
        let view = PopupView {
            popup: &popup,
            lang: Lang::Ko,
        };

        terminal
            .draw(|frame| view.render(frame, frame.area()))
            .unwrap();

        let buffer = terminal.backend().buffer();
        let row: String = (0..30).map(|x| buffer[(x, 0)].symbol()).collect();
        assert!(row.starts_with("› help"));
        assert!(row.ends_with("Saturn"));
    }
}
