//! 사용량 화면(`/usage`). 범위 `chat`, `today`, `week`, `all`.
//!
//! 설계: docs/design/tui.md(영역 사용량 화면, 키 사용량 화면), docs/design/providers-and-sessions.md(사용량 보고).
//! 보고하지 않은 값은 0으로 채우지 않고 `-`로 보인다.
//! `Request::Usage`의 응답은 `Notification::Usage`이고, 행은 protocol `UsageRow`를 그대로 쓴다.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use saturn_protocol::rpc::{UsageRange, UsageRow};

use crate::i18n::{self, Lang};
use crate::view::{EMPHASIS, MUTED, text_width, truncate, window_block};

/// 토큰 칸 폭.
const NUMBER_COLS: usize = 11;

/// 사용량 화면 내용.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageTable {
    /// 범위.
    pub range: UsageRange,
    /// 에이전트별 행과 judge 행.
    pub rows: Vec<UsageRow>,
}

/// 사용량 화면 상태.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageScreen {
    /// 받은 내용. 응답 전이면 `None`.
    pub table: Option<UsageTable>,
    /// 요청한 범위.
    pub range: UsageRange,
    /// `Enter`로 judge 실제 모델 상세를 펼쳤다.
    pub detail: bool,
}

impl UsageScreen {
    /// 범위로 연다(응답 대기).
    pub fn new(range: UsageRange) -> Self {
        Self {
            table: None,
            range,
            detail: false,
        }
    }

    /// `Enter` judge 실제 모델 상세를 펼치거나 접는다.
    pub fn toggle_detail(&mut self) {
        self.detail = !self.detail;
    }
}

/// 사용량 화면 그리기.
#[derive(Debug)]
pub struct UsageView<'a> {
    /// 화면 상태.
    pub screen: &'a UsageScreen,
    /// 화면 언어.
    pub lang: Lang,
}

impl UsageView<'_> {
    // cost: time O(r·w), heap O(r·w), stack O(1)
    // vars: r = 행 수, w = 칸 폭
    // basis: estimate
    /// 범위 머리, 에이전트별 새 입력·캐시 읽기·캐시 쓰기·출력·추론 표(천 단위 쉼표, 미보고 `-`),
    /// judge 호출과 예상 비용, 맥락 정리, 채점 합계 줄, 상세면 judge를 부른 행(`who`가 judge 실제 모델)별 호출과 비용을 그린다.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let lang = self.lang;
        let title = format!(
            "{} · {}",
            lang.tr(i18n::USAGE_TITLE),
            range_name(self.screen.range)
        );
        let block = window_block(&title);
        let inner = block.inner(area);
        frame.render_widget(Clear, area);
        frame.render_widget(block, area);
        let width = usize::from(inner.width);
        let lines = match &self.screen.table {
            None => vec![Line::from(Span::styled(lang.tr(i18n::LOADING), MUTED))],
            Some(table) => table_lines(lang, table, self.screen.detail),
        };
        let lines: Vec<Line> = lines
            .into_iter()
            .map(|line| {
                let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
                let style = line.spans.first().map(|s| s.style).unwrap_or_default();
                Line::from(Span::styled(truncate(&text, width), style))
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), inner);
    }
}

/// 범위 이름(`chat`, `today`, `week`, `all`). 명령 인자와 같아 번역하지 않는다.
pub fn range_name(range: UsageRange) -> &'static str {
    match range {
        UsageRange::Chat => "chat",
        UsageRange::Today => "today",
        UsageRange::Week => "week",
        UsageRange::All => "all",
    }
}

/// 예상 비용(마이크로 달러)을 `$0.0123`으로. 모르면 `-`.
pub fn cost_text(micros: Option<u64>) -> String {
    match micros {
        Some(micros) => format!("${:.4}", micros as f64 / 1_000_000.0),
        None => "-".to_string(),
    }
}

// cost: time O(r), heap O(r), stack O(1)
// vars: r = table.rows.len()
// basis: estimate
/// 표 줄: 머리, 행, 합계, 상세.
fn table_lines(lang: Lang, table: &UsageTable, detail: bool) -> Vec<Line<'static>> {
    let who_width = table
        .rows
        .iter()
        .map(|row| text_width(&row.who))
        .chain([text_width(lang.tr(i18n::USAGE_WHO))])
        .max()
        .unwrap_or(0);
    let headers = [
        i18n::USAGE_INPUT,
        i18n::USAGE_CACHE_READ,
        i18n::USAGE_CACHE_WRITE,
        i18n::USAGE_OUTPUT,
        i18n::USAGE_REASONING,
    ]
    .map(|key| lang.tr(key).to_string());
    let mut lines = vec![Line::from(Span::styled(
        table_row(lang.tr(i18n::USAGE_WHO), &headers, who_width),
        EMPHASIS,
    ))];
    lines.extend(table.rows.iter().map(|row| {
        let cells = row.tokens.map(|value| match value {
            Some(value) => i18n::format_count(value),
            None => "-".to_string(),
        });
        Line::from(table_row(&row.who, &cells, who_width))
    }));
    lines.push(Line::from(""));
    lines.push(Line::from(totals_line(lang, &table.rows)));
    if detail {
        lines.extend(
            table
                .rows
                .iter()
                .filter(|row| row.judge_calls > 0)
                .map(|row| {
                    Line::from(Span::styled(
                        format!(
                            "  {} · {} {} · {} {}",
                            row.who,
                            lang.tr(i18n::USAGE_JUDGE_CALLS),
                            row.judge_calls,
                            lang.tr(i18n::USAGE_COST),
                            cost_text(row.estimated_cost_micros)
                        ),
                        MUTED,
                    ))
                }),
        );
    }
    lines
}

// cost: time O(1), heap O(1), stack O(1)
// basis: estimate
/// 표 한 행. 대상은 왼쪽 정렬, 숫자는 오른쪽 정렬.
fn table_row(who: &str, cells: &[String; 5], who_width: usize) -> String {
    let mut row = format!("{who}{}", " ".repeat(who_width - text_width(who)));
    for cell in cells {
        let pad = NUMBER_COLS.saturating_sub(text_width(cell));
        row.push_str(&" ".repeat(pad));
        row.push_str(cell);
    }
    row
}

// cost: time O(r), heap O(r), stack O(1)
// vars: r = 행 수
// basis: estimate
/// 합계 줄 `판단기 호출 3 · 예상 비용 $0.0120 · 맥락 정리 1 · 채점 40`. 비용은 아는 값만 더한다.
fn totals_line(lang: Lang, rows: &[UsageRow]) -> String {
    let calls: u32 = rows.iter().map(|row| row.judge_calls).sum();
    let costs: Vec<u64> = rows
        .iter()
        .filter_map(|row| row.estimated_cost_micros)
        .collect();
    let cost = (!costs.is_empty()).then(|| costs.iter().sum());
    let compactions: u32 = rows.iter().map(|row| row.compactions).sum();
    let labels: u32 = rows.iter().map(|row| row.labels).sum();
    format!(
        "{} {calls} · {} {} · {} {compactions} · {} {labels}",
        lang.tr(i18n::USAGE_JUDGE_CALLS),
        lang.tr(i18n::USAGE_COST),
        cost_text(cost),
        lang.tr(i18n::USAGE_COMPACTIONS),
        lang.tr(i18n::USAGE_LABELS)
    )
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn row(who: &str, tokens: [Option<u64>; 5], judge_calls: u32) -> UsageRow {
        UsageRow {
            who: who.to_string(),
            tokens,
            judge_calls,
            estimated_cost_micros: (judge_calls > 0).then_some(12_000),
            compactions: 1,
            labels: 0,
        }
    }

    fn screen(detail: bool) -> UsageScreen {
        UsageScreen {
            table: Some(UsageTable {
                range: UsageRange::Chat,
                rows: vec![
                    row("A codex", [Some(1_200), None, None, Some(300), None], 0),
                    row("judge-model", [None; 5], 3),
                ],
            }),
            range: UsageRange::Chat,
            detail,
        }
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    fn content(screen: &UsageScreen) -> String {
        let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
        let view = UsageView {
            screen,
            lang: Lang::En,
        };
        terminal
            .draw(|frame| view.render(frame, frame.area()))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn render_shows_unreported_as_dash_and_totals() {
        let content = content(&screen(false));

        assert!(content.contains("1,200"));
        assert!(content.contains("-"));
        assert!(content.contains("judge calls 3 · estimated cost $0.0120 · compactions 2"));
        assert!(!content.contains("  judge-model ·"));
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn toggle_detail_shows_judge_rows() {
        let mut screen = screen(false);

        screen.toggle_detail();

        assert!(content(&screen).contains("judge-model · judge calls 3"));
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn render_before_response_shows_loading() {
        assert!(content(&UsageScreen::new(UsageRange::Week)).contains("loading"));
    }
}
