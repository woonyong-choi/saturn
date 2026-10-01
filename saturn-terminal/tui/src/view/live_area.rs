//! 작업별 출력 칸. 출력이 흐르는 작업마다 최근 줄을 보인다.
//! 설계: docs/design/tui.md

use std::collections::VecDeque;

use ratatui::Frame;
use ratatui::layout::Rect;
use saturn_protocol::ids::{TaskId, TaskLabel};

use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::i18n::Lang;
use crate::labels;
use crate::view::truncate;

pub const MAX_ROWS: u16 = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveTask {
    pub task: TaskId,
    pub label: TaskLabel,
    pub lines: Vec<String>,
    /// 아직 줄바꿈이 오지 않은 조각.
    pub partial: String,
}

/// 출력이 시작된 순서이며 그 뒤에는 바꾸지 않는다.
#[derive(Debug, Default)]
pub struct LiveArea {
    tasks: VecDeque<LiveTask>,
}

impl LiveArea {
    pub fn new() -> Self {
        Self::default()
    }

    // cost: time O(k + n), heap O(n), stack O(1)
    // vars: k = 칸의 작업 수, n = 조각 길이
    // basis: estimate
    /// 완성된 줄이 새로 생겼으면 `true`.
    pub fn push(&mut self, task: TaskId, label: TaskLabel, text: &str) -> bool {
        let index = match self.tasks.iter().position(|t| t.task == task) {
            Some(index) => index,
            None => {
                self.tasks.push_back(LiveTask {
                    task,
                    label,
                    lines: Vec::new(),
                    partial: String::new(),
                });
                self.tasks.len() - 1
            }
        };
        let entry = &mut self.tasks[index];
        entry.label = label;
        entry.partial.push_str(text);
        let mut completed = false;
        while let Some(end) = entry.partial.find('\n') {
            let line: String = entry.partial.drain(..=end).collect();
            entry
                .lines
                .push(line.trim_end_matches(['\n', '\r']).to_string());
            completed = true;
        }
        completed
    }

    // cost: time O(k), heap O(1), stack O(1)
    // vars: k = 칸의 작업 수
    // basis: estimate
    /// 남은 조각까지 줄로 돌려주고 칸에서 지운다.
    pub fn finish(&mut self, task: TaskId) -> Vec<String> {
        let Some(index) = self.tasks.iter().position(|t| t.task == task) else {
            return Vec::new();
        };
        let Some(mut entry) = self.tasks.remove(index) else {
            return Vec::new();
        };
        if !entry.partial.is_empty() {
            entry.lines.push(std::mem::take(&mut entry.partial));
        }
        entry.lines
    }

    // cost: time O(k), heap O(1), stack O(1)
    // vars: k = 칸의 작업 수
    // basis: estimate
    /// `max_rows(screen_height)`를 넘지 않는다.
    pub fn height(&self, screen_height: u16) -> u16 {
        let wanted: usize = self.tasks.iter().map(|t| t.lines.len()).sum();
        let wanted = u16::try_from(wanted).unwrap_or(u16::MAX);
        wanted.min(max_rows(screen_height))
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    pub fn tasks(&self) -> impl ExactSizeIterator<Item = &LiveTask> {
        self.tasks.iter()
    }
}

/// 화면 높이의 1/4이 `MAX_ROWS`보다 작으면 그 값(최소 1줄, 초안).
pub fn max_rows(screen_height: u16) -> u16 {
    (screen_height / 4).clamp(1, MAX_ROWS)
}

#[derive(Debug)]
pub struct LiveAreaView<'a> {
    pub live: &'a LiveArea,
    pub lang: Lang,
    pub labels_visible: bool,
}

impl LiveAreaView<'_> {
    // cost: time O(r·k + w·r), heap O(w·r), stack O(1)
    // vars: r = 칸 높이, k = 칸의 작업 수, w = 칸 폭
    // basis: estimate
    /// 줄 수가 모자라면 작업마다 가장 최근 줄부터 남긴다.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let budgets = row_budgets(self.live, usize::from(area.height));
        let width = usize::from(area.width);
        let mut rows: Vec<Line> = Vec::new();
        for (task, budget) in self.live.tasks().zip(budgets) {
            let prefix = labels::prefix(Some(task.label), self.labels_visible);
            let start = task.lines.len().saturating_sub(budget);
            rows.extend(
                task.lines[start..]
                    .iter()
                    .map(|line| Line::from(truncate(&format!("{prefix}{line}"), width))),
            );
        }
        frame.render_widget(Paragraph::new(rows), area);
    }
}

// cost: time O(rows · k), heap O(k), stack O(1)
// vars: k = 칸에 있는 작업 수
// basis: estimate
/// 줄이 있는 작업에 한 줄씩 돌아가며 나눈다.
fn row_budgets(live: &LiveArea, rows: usize) -> Vec<usize> {
    let available: Vec<usize> = live.tasks().map(|t| t.lines.len()).collect();
    let mut budgets = vec![0; available.len()];
    let mut left = rows;
    while left > 0 {
        let mut gave = false;
        for (budget, available) in budgets.iter_mut().zip(&available) {
            if left > 0 && *budget < *available {
                *budget += 1;
                left -= 1;
                gave = true;
            }
        }
        if !gave {
            break;
        }
    }
    budgets
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    #[test]
    fn push_completes_only_on_newline() {
        let mut live = LiveArea::new();

        let partial = live.push(TaskId(1), TaskLabel('A'), "hel");
        let complete = live.push(TaskId(1), TaskLabel('A'), "lo\nwor");

        assert!(!partial);
        assert!(complete);
        let task = live.tasks().next().unwrap();
        assert_eq!(task.lines, vec!["hello"]);
        assert_eq!(task.partial, "wor");
    }

    #[test]
    fn finish_returns_all_lines_and_removes_task() {
        let mut live = LiveArea::new();
        live.push(TaskId(1), TaskLabel('A'), "a\nb");

        let lines = live.finish(TaskId(1));

        assert_eq!(lines, vec!["a", "b"]);
        assert_eq!(live.tasks().len(), 0);
        assert!(live.finish(TaskId(1)).is_empty());
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn push_keeps_first_output_order() {
        let mut live = LiveArea::new();
        live.push(TaskId(2), TaskLabel('B'), "b\n");
        live.push(TaskId(1), TaskLabel('A'), "a\n");

        live.push(TaskId(2), TaskLabel('B'), "b2\n");

        let order: Vec<TaskId> = live.tasks().map(|t| t.task).collect();
        assert_eq!(order, vec![TaskId(2), TaskId(1)]);
    }

    #[test]
    fn max_rows_caps_at_eight_and_shrinks_on_small_screens() {
        assert_eq!(max_rows(80), 8);
        assert_eq!(max_rows(20), 5);
        assert_eq!(max_rows(2), 1);
    }

    #[test]
    fn height_is_capped_by_max_rows() {
        let mut live = LiveArea::new();
        live.push(TaskId(1), TaskLabel('A'), &"x\n".repeat(20));

        assert_eq!(live.height(80), 8);
        assert_eq!(live.height(12), 3);
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn render_splits_rows_and_keeps_latest_lines() {
        let mut live = LiveArea::new();
        live.push(TaskId(1), TaskLabel('A'), "a1\na2\na3\n");
        live.push(TaskId(2), TaskLabel('B'), "b1\n");
        let mut terminal = Terminal::new(TestBackend::new(10, 3)).unwrap();
        let view = LiveAreaView {
            live: &live,
            lang: Lang::Ko,
            labels_visible: true,
        };

        terminal
            .draw(|frame| view.render(frame, frame.area()))
            .unwrap();

        let buffer = terminal.backend().buffer();
        let rows: Vec<String> = (0..3)
            .map(|y| {
                let row: String = (0..10).map(|x| buffer[(x, y)].symbol()).collect();
                row.trim_end().to_string()
            })
            .collect();
        assert_eq!(rows, vec!["[A] a2", "[A] a3", "[B] b1"]);
    }
}
