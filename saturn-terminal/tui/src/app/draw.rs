//! 화면 전체 그리기. 영역 배치와 창 덮기.
//!
//! 설계: docs/design/tui.md(배치). 위에서 아래로 시작 화면 또는 대화 기록, 작업별 출력 칸, 상태판, 팝업, 입력창, 바닥줄.
//! 창은 그 위에 덮고, 허가 요청 창은 일반 창 위에, judge 키·폴더 신뢰 창은 맨 위에 그린다.

use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;

use super::{App, Window};
use crate::i18n;
use crate::view::composer::ComposerView;
use crate::view::folder_trust::FolderTrustView;
use crate::view::footer::FooterView;
use crate::view::full_transcript::FullTranscriptView;
use crate::view::judge_key_prompt::JudgeKeyPromptView;
use crate::view::judge_version::JudgeVersionView;
use crate::view::live_area::LiveAreaView;
use crate::view::permission::PermissionView;
use crate::view::popup::PopupView;
use crate::view::resume_prompt::ResumePromptView;
use crate::view::start_screen::StartScreenView;
use crate::view::status_board::{self, StatusBoardView};
use crate::view::task_list::TaskListView;
use crate::view::train_confirm::TrainConfirmView;
use crate::view::transcript::TranscriptView;
use crate::view::usage::UsageView;
use crate::view::{self, Areas, Heights, render_window};

impl App {
    // cost: time O(c + w·h), heap O(c + w·h), stack O(1)
    // vars: c = 대화 기록 글자 수, w·h = 화면 칸 수
    // basis: estimate
    /// 화면 전체를 그린다. `view::layout`으로 영역을 나누고 시작 화면 또는 대화 기록, 작업별 출력 칸, 상태판,
    /// 팝업, 입력창, 바닥줄을 그린 뒤 창(`window`)과 허가 요청 창을 덮는다. 전체 기록은 화면 전체를 쓴다.
    pub fn render(&self, frame: &mut Frame, now: Instant) {
        let area = frame.area();
        let lang = self.lang;
        let labels_visible = self.chat.labels_visible();
        if let Some(Window::FullTranscript(state)) = &self.window {
            let view = FullTranscriptView {
                state,
                transcript: &self.transcript,
                lang,
                labels_visible,
            };
            view.render(frame, area);
            return;
        }
        let lines = status_board::build(&self.chat, now);
        let areas = self.areas(area, lines.len());
        match &self.start {
            Some(info) => StartScreenView { info, lang }.render(frame, areas.transcript),
            None => TranscriptView {
                transcript: &self.transcript,
                lang,
                labels_visible,
            }
            .render(frame, areas.transcript),
        }
        let live = LiveAreaView {
            live: &self.live,
            lang,
            labels_visible,
        };
        live.render(frame, areas.live);
        let board = StatusBoardView {
            lines: &lines,
            lang,
            labels_visible,
            spinner: view::spinner(self.tick),
        };
        board.render(frame, areas.status);
        if let Some(popup) = &self.popup {
            PopupView { popup, lang }.render(frame, areas.popup);
        }
        let composer = ComposerView {
            composer: &self.composer,
            lang,
            search_result: self.search_result(),
        };
        composer.render(frame, areas.composer);
        let footer = FooterView {
            lang,
            context: self.chat.context,
        };
        footer.render(frame, areas.footer);
        self.render_windows(frame, area, now);
    }

    /// 화면 영역. 상태판은 줄 수만큼, 작업별 출력 칸은 `LiveArea::height`만큼.
    pub(super) fn areas(&self, area: Rect, status_lines: usize) -> Areas {
        let composer = if self.composer.search().is_some() {
            1
        } else {
            self.composer.height()
        };
        view::layout(
            area,
            Heights {
                live: self.live.height(area.height),
                status: u16::try_from(status_lines).unwrap_or(u16::MAX),
                popup: self.popup.as_ref().map_or(0, |popup| popup.height()),
                composer,
            },
        )
    }

    /// 창 덮기. 일반 창 → 허가 요청 창 → judge 키·폴더 신뢰 창 순서로 위에 그린다.
    fn render_windows(&self, frame: &mut Frame, area: Rect, now: Instant) {
        let lang = self.lang;
        match &self.window {
            Some(Window::Resume(prompt)) => ResumePromptView { prompt, lang }.render(frame, area),
            Some(Window::TaskList(list)) => TaskListView { list, lang }.render(frame, area),
            Some(Window::Usage(screen)) => UsageView { screen, lang }.render(frame, area),
            Some(Window::JudgeVersion(screen)) => {
                JudgeVersionView { screen, lang }.render(frame, area);
            }
            Some(Window::TrainConfirm(confirm)) => {
                TrainConfirmView { confirm, lang }.render(frame, area);
            }
            Some(Window::Shortcuts) => render_shortcuts(self, frame, area),
            _ => {}
        }
        if !self.permissions.is_empty() {
            let view = PermissionView {
                queue: &self.permissions,
                lang,
                guarded: !self.permissions.accepts_input(now),
            };
            view.render(frame, area);
        }
        match &self.window {
            Some(Window::JudgeKey(prompt)) => {
                JudgeKeyPromptView { prompt, lang }.render(frame, area);
            }
            Some(Window::FolderTrust(trust)) => FolderTrustView { trust, lang }.render(frame, area),
            _ => {}
        }
    }
}

// cost: time O(1), heap O(1), stack O(1)
// basis: estimate
/// `?` 단축키 안내 창. 키 칸을 맞춰 설명을 붙인다.
fn render_shortcuts(app: &App, frame: &mut Frame, area: Rect) {
    let lang = app.lang;
    let width = i18n::SHORTCUTS
        .iter()
        .map(|(key, _)| view::text_width(key))
        .max()
        .unwrap_or(0);
    let lines: Vec<Line> = i18n::SHORTCUTS
        .iter()
        .map(|(key, desc)| {
            let pad = " ".repeat(width - view::text_width(key));
            Line::from(format!("{key}{pad}  {}", lang.tr(desc)))
        })
        .collect();
    render_window(frame, area, lang.tr(i18n::SHORTCUTS_TITLE), lines);
}
