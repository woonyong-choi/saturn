//! 화면 전체 그리기. 영역 배치와 창 덮기.
//! 설계: docs/design/tui.md

use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::text::Line;

use super::{App, Window};
use crate::i18n;
use crate::view::composer::ComposerView;
use crate::view::constraint_ask::ConstraintAskView;
use crate::view::exit_confirm::ExitConfirmView;
use crate::view::folder_trust::FolderTrustView;
use crate::view::footer::FooterView;
use crate::view::full_transcript::FullTranscriptView;
use crate::view::input_request::InputView;
use crate::view::live_area::LiveAreaView;
use crate::view::model_picker::ModelPickerView;
use crate::view::permission::PermissionView;
use crate::view::popup::PopupView;
use crate::view::prune_window::PruneWindowView;
use crate::view::resume_prompt::ResumePromptView;
use crate::view::router_key_prompt::RouterKeyPromptView;
use crate::view::router_version::RouterVersionView;
use crate::view::start_screen::StartScreenView;
use crate::view::status_board::{self, StatusBoardView};
use crate::view::stop_confirm::StopConfirmView;
use crate::view::task_list::TaskListView;
use crate::view::transcript::TranscriptView;
use crate::view::usage::UsageView;
use crate::view::{self, Areas, Heights, render_window};

impl App {
    // cost: time O(c + w·h), heap O(c + w·h), stack O(1)
    // vars: c = 대화 기록 글자 수, w·h = 화면 칸 수
    // basis: estimate
    pub(crate) fn render(&self, frame: &mut Frame, now: Instant) {
        view::set_plain(self.plain);
        self.render_frame(frame, now);
        if self.plain {
            // 색 없이 글자 속성만 쓴다. `NO_COLOR`를 따른다
            for cell in &mut frame.buffer_mut().content {
                cell.set_fg(Color::Reset);
            }
        }
    }

    fn render_frame(&self, frame: &mut Frame, now: Instant) {
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
        let board = status_board::board(&self.chat, now);
        let areas = self.areas(
            area,
            board.as_ref().map_or(0, |board| board.height(area.width)),
        );
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
            labels_visible,
        };
        live.render(frame, areas.live);
        let status = StatusBoardView {
            board: board.as_ref(),
            lang,
            labels_visible,
            spinner: view::spinner(self.tick),
            focus: self.board_focus,
        };
        status.render(frame, areas.status);
        if let Some(popup) = &self.popup {
            PopupView { popup }.render(frame, areas.popup);
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
            model_mode: self.chat.model_mode,
        };
        footer.render(frame, areas.footer);
        self.render_windows(frame, area, now);
    }

    pub(super) fn areas(&self, area: Rect, status_rows: u16) -> Areas {
        let composer = if self.composer.search().is_some() {
            1
        } else {
            self.composer.height()
        };
        view::layout(
            area,
            Heights {
                live: self.live.height(area.height),
                status: status_rows,
                popup: self.popup.as_ref().map_or(0, |popup| popup.height()),
                composer,
            },
        )
    }

    fn render_windows(&self, frame: &mut Frame, area: Rect, now: Instant) {
        let lang = self.lang;
        match &self.window {
            Some(Window::Resume(prompt)) => ResumePromptView { prompt, lang }.render(frame, area),
            Some(Window::TaskList(list)) => TaskListView { list, lang }.render(frame, area),
            Some(Window::Usage(screen)) => UsageView { screen, lang }.render(frame, area),
            Some(Window::RouterVersion(screen)) => {
                RouterVersionView { screen, lang }.render(frame, area);
            }
            Some(Window::StopConfirm(confirm)) => {
                StopConfirmView { confirm, lang }.render(frame, area);
            }
            Some(Window::ConstraintAsk(ask)) => ConstraintAskView {
                ask,
                waiting: self.constraint_asks.waiting_besides(ask.ask),
                lang,
            }
            .render(frame, area),
            Some(Window::Model(picker)) => ModelPickerView { picker, lang }.render(frame, area),
            Some(Window::Prune(window)) => PruneWindowView { window, lang }.render(frame, area),
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
        } else if !self.inputs.is_empty() {
            let view = InputView {
                queue: &self.inputs,
                lang,
                guarded: !self.inputs.accepts_input(now),
            };
            view.render(frame, area);
        }
        match &self.window {
            Some(Window::RouterKey(prompt)) => {
                RouterKeyPromptView { prompt, lang }.render(frame, area);
            }
            Some(Window::FolderTrust(trust)) => FolderTrustView { trust, lang }.render(frame, area),
            _ => {}
        }
        if let Some(confirm) = &self.exit_confirm {
            ExitConfirmView { confirm, lang }.render(frame, area);
        }
    }
}

// cost: time O(1), heap O(1), stack O(1)
// basis: estimate
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
