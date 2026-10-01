//! `App`의 키 처리 상태 전이, 알림 반영, 전체 화면 그리기 테스트.

use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::{AgentId, ChatId, InputId, JudgmentId, Provider, TaskId, TaskLabel};
use saturn_protocol::rpc::{Alert, Notification, PermissionAnswer, Request, UsageRange};
use saturn_protocol::state::{Disposition, InputState, QueueReason, TaskState};

use super::{App, AppEvent, Effect, Window};
use crate::history::InputHistory;
use crate::i18n::Lang;
use crate::keys::KeyArea;
use crate::view::buffer_lines;
use crate::view::popup::PopupKind;
use crate::view::transcript::TranscriptCell;

// cost: time O(1), heap O(1), stack O(1)
// basis: estimate
fn app() -> App {
    let history =
        InputHistory::load(&std::env::temp_dir().join("saturn-tui-app-test-missing")).unwrap();
    let mut app = App::new(Lang::Ko, "/work".into(), history, None);
    app.screen = Rect::new(0, 0, 60, 20);
    app
}

fn attached() -> App {
    let mut app = app();
    notify(&mut app, history(Vec::new()));
    app
}

fn history(entries: Vec<Notification>) -> Notification {
    Notification::HistoryChunk {
        chat: ChatId(7),
        entries,
        has_more: false,
    }
}

fn notify(app: &mut App, notification: Notification) -> Vec<Effect> {
    app.handle(AppEvent::Engine(notification), Instant::now())
}

fn press(app: &mut App, code: KeyCode, modifiers: KeyModifiers) -> Vec<Effect> {
    let key = KeyEvent::new(code, modifiers);
    app.handle(AppEvent::Terminal(Event::Key(key)), Instant::now())
}

fn press_at(app: &mut App, code: KeyCode, now: Instant) -> Vec<Effect> {
    let key = KeyEvent::new(code, KeyModifiers::NONE);
    app.handle(AppEvent::Terminal(Event::Key(key)), now)
}

// cost: time O(1), heap O(1), stack O(1)
// basis: estimate
fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        press(app, KeyCode::Char(c), KeyModifiers::NONE);
    }
}

fn task(id: u64, label: char, state: TaskState) -> Notification {
    Notification::TaskChanged {
        task: TaskId(id),
        label: TaskLabel(label),
        state,
        provider: Some(Provider::Codex),
        elapsed_ms: 45_000,
        failure: None,
    }
}

fn input(id: u64, state: InputState, text: &str) -> Notification {
    Notification::InputChanged {
        input: InputId(id),
        text: text.to_string(),
        label: Some(TaskLabel('C')),
        state,
        disposition: None,
        reason: (state == InputState::Queued).then_some(QueueReason::WriteTurn),
    }
}

// cost: time O(1), heap O(1), stack O(1)
// basis: estimate
fn sent(effects: &[Effect]) -> Vec<&Request> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Send(request) => Some(request),
            _ => None,
        })
        .collect()
}

#[test]
fn attach_request_carries_chat_workdir_and_env() {
    let mut app = App::new(
        Lang::Ko,
        "/work".into(),
        InputHistory::with_entries(&[]),
        Some(ChatId(3)),
    );
    app.env = vec![("PATH".to_string(), "/usr/bin".to_string())];

    assert_eq!(
        app.attach_request(),
        Request::Attach {
            chat: Some(ChatId(3)),
            workdir: "/work".to_string(),
            env: vec![("PATH".to_string(), "/usr/bin".to_string())],
            overrides: Vec::new(),
        }
    );
}

#[test]
fn enter_idle_submits_input_and_records_history() {
    let mut app = attached();
    type_text(&mut app, "버그 고쳐");

    let effects = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    assert_eq!(
        effects,
        vec![
            Effect::RecordHistory("버그 고쳐".to_string()),
            Effect::Send(Request::SubmitInput {
                chat: ChatId(7),
                client_ref: 1,
                text: "버그 고쳐".to_string(),
                pinned_model: None,
                skip_relation: false,
            }),
        ]
    );
    assert!(app.composer.is_empty());
}

#[test]
fn tab_while_running_queues_without_relation_judgment() {
    let mut app = attached();
    notify(&mut app, task(1, 'A', TaskState::Running));
    type_text(&mut app, "테스트도");

    let effects = press(&mut app, KeyCode::Tab, KeyModifiers::NONE);

    assert!(matches!(
        sent(&effects)[0],
        Request::SubmitInput {
            skip_relation: true,
            ..
        }
    ));
}

#[test]
fn submit_before_chat_known_keeps_draft() {
    let mut app = app();
    type_text(&mut app, "hi");

    let effects = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    assert!(effects.is_empty());
    assert_eq!(app.composer.text(), "hi");
}

#[test]
fn submit_while_intake_stopped_keeps_draft() {
    let mut app = attached();
    notify(
        &mut app,
        Notification::Alert {
            alert: Alert::IntakeStopped,
        },
    );
    type_text(&mut app, "hi");

    let effects = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    assert!(effects.is_empty());
    assert_eq!(app.composer.text(), "hi");
}

// cost: time O(1), heap O(1), stack O(1)
// basis: estimate
#[test]
fn shell_line_runs_shell_and_attaches_result_to_next_input() {
    let mut app = attached();
    type_text(&mut app, "!ls");

    let effects = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    app.handle(
        AppEvent::ShellDone(crate::shell::ShellOutput {
            command: "ls".to_string(),
            status: Some(0),
            output: "a".to_string(),
        }),
        Instant::now(),
    );
    type_text(&mut app, "봐줘");
    let next = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    assert!(effects.contains(&Effect::RunShell("ls".to_string())));
    assert!(matches!(
        sent(&next)[0],
        Request::SubmitInput { text, .. } if text == "봐줘\n\n$ ls\na\n[exit 0]"
    ));
    assert!(app.pending_attachments.is_empty());
}

#[test]
fn usage_command_opens_screen_and_requests_rows() {
    let mut app = attached();
    type_text(&mut app, "/usage week");
    app.popup = None;

    let effects = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    assert_eq!(
        sent(&effects),
        vec![&Request::Usage {
            scope: UsageRange::Week
        }]
    );
    assert!(matches!(app.window, Some(Window::Usage(_))));
}

#[test]
fn invalid_command_adds_warning_and_keeps_draft() {
    let mut app = attached();
    type_text(&mut app, "/usage year");
    app.popup = None;

    let effects = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    assert!(effects.is_empty());
    assert_eq!(app.composer.text(), "/usage year");
    assert_eq!(
        app.transcript.cells().last(),
        Some(&TranscriptCell::Warning(
            "잘못된 인자: /usage year".to_string()
        ))
    );
}

#[test]
fn ctrl_c_clears_draft_then_stops_then_quits() {
    let mut app = attached();
    notify(&mut app, task(1, 'A', TaskState::Running));
    type_text(&mut app, "draft");

    let first = press(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
    let second = press(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
    notify(&mut app, task(1, 'A', TaskState::Held));
    let third = press(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);

    assert!(first.is_empty());
    assert!(app.composer.is_empty());
    assert_eq!(sent(&second), vec![&Request::Stop { chat: ChatId(7) }]);
    assert_eq!(third, vec![Effect::Quit]);
}

#[test]
fn esc_closes_task_list() {
    let mut app = attached();
    type_text(&mut app, "/tasks");
    app.popup = None;
    let effects = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    press(&mut app, KeyCode::Esc, KeyModifiers::NONE);

    assert_eq!(sent(&effects), vec![&Request::ListTasks]);
    assert!(app.window.is_none());
}

#[test]
fn ctrl_c_closes_full_transcript_before_stopping() {
    let mut app = attached();
    notify(&mut app, task(1, 'A', TaskState::Running));
    press(&mut app, KeyCode::Char('t'), KeyModifiers::CONTROL);

    let effects = press(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);

    assert!(effects.is_empty());
    assert!(app.window.is_none());
}

#[test]
fn permission_window_ignores_keys_for_one_second() {
    let mut app = attached();
    let now = Instant::now();
    app.handle(
        AppEvent::Engine(Notification::PermissionRequested {
            task: TaskId(1),
            label: TaskLabel('A'),
            provider: Provider::Codex,
            request_id: "r1".to_string(),
            summary: "rm".to_string(),
            reason: "clean".to_string(),
            waiting: 0,
        }),
        now,
    );

    let early = press_at(
        &mut app,
        KeyCode::Char('y'),
        now + Duration::from_millis(300),
    );
    let late = press_at(&mut app, KeyCode::Char('y'), now + Duration::from_secs(1));

    assert_eq!(app.key_area(), KeyArea::Composer);
    assert!(early.is_empty());
    assert_eq!(
        sent(&late),
        vec![&Request::AnswerPermission {
            request_id: "r1".to_string(),
            answer: PermissionAnswer::Allow,
        }]
    );
}

#[test]
fn permission_resolved_elsewhere_closes_window() {
    let mut app = attached();
    notify(
        &mut app,
        Notification::PermissionRequested {
            task: TaskId(1),
            label: TaskLabel('A'),
            provider: Provider::Codex,
            request_id: "r1".to_string(),
            summary: "rm".to_string(),
            reason: "clean".to_string(),
            waiting: 0,
        },
    );
    let area = app.key_area();

    notify(
        &mut app,
        Notification::PermissionResolved {
            request_id: "r1".to_string(),
        },
    );

    assert_eq!(area, KeyArea::Permission);
    assert!(app.permissions.is_empty());
}

fn feedback(app: &mut App, now: Instant) {
    app.handle(
        AppEvent::Engine(Notification::FeedbackQuestion {
            judgment: JudgmentId(9),
            input: InputId(1),
            label: TaskLabel('A'),
            disposition: Disposition::Steer,
        }),
        now,
    );
}

#[test]
fn feedback_no_with_unsent_input_offers_correction() {
    let mut app = attached();
    notify(&mut app, input(1, InputState::Queued, "x"));
    feedback(&mut app, Instant::now());
    let area = app.key_area();

    let effects = press(&mut app, KeyCode::Char('2'), KeyModifiers::NONE);

    assert_eq!(area, KeyArea::Transcript);
    assert_eq!(
        sent(&effects),
        vec![&Request::AnswerFeedback {
            judgment: JudgmentId(9),
            correct: false,
        }]
    );
    assert_eq!(
        app.transcript.cells().last(),
        Some(&TranscriptCell::Correction {
            label: TaskLabel('A')
        })
    );
}

#[test]
fn feedback_area_passes_letters_to_composer() {
    let mut app = attached();
    feedback(&mut app, Instant::now());

    press(&mut app, KeyCode::Char('h'), KeyModifiers::NONE);

    assert_eq!(app.composer.text(), "h");
    assert!(app.chat.feedback.is_some());
}

// cost: time O(1), heap O(1), stack O(1)
// basis: estimate
#[test]
fn feedback_disappears_after_eight_seconds() {
    let mut app = attached();
    let now = Instant::now();
    feedback(&mut app, now);

    app.handle(AppEvent::Tick, now + Duration::from_secs(7));
    let still = app.chat.feedback.is_some();
    app.handle(AppEvent::Tick, now + Duration::from_secs(8));

    assert!(still);
    assert!(app.chat.feedback.is_none());
    assert!(
        !app.transcript
            .cells()
            .iter()
            .any(|cell| matches!(cell, TranscriptCell::Feedback { .. }))
    );
}

// cost: time O(1), heap O(1), stack O(1)
// basis: estimate
#[test]
fn notifications_build_echo_live_output_and_result() {
    let mut app = attached();
    notify(
        &mut app,
        Notification::StartInfo {
            saturn_version: "0.1.0".to_string(),
            providers: Vec::new(),
            judge: String::new(),
            judge_version: String::new(),
            folder: "/work".to_string(),
        },
    );
    notify(&mut app, input(1, InputState::Delivering, "버그 고쳐"));
    notify(&mut app, task(1, 'A', TaskState::Running));
    notify(
        &mut app,
        Notification::TaskEvent {
            task: TaskId(1),
            event: ProviderEvent::Text {
                agent: AgentId(1),
                subagent: None,
                text: "고쳤습니다\n".to_string(),
            },
        },
    );
    let live_rows = app.live.tasks().len();

    notify(&mut app, task(1, 'A', TaskState::Done));

    assert!(app.start.is_none());
    assert_eq!(live_rows, 1);
    assert_eq!(app.live.tasks().len(), 0);
    let lines: Vec<String> = app
        .transcript
        .cells()
        .iter()
        .flat_map(|cell| cell.lines(Lang::Ko, false, false))
        .collect();
    assert_eq!(
        &lines[lines.len() - 3..],
        &[
            "> 버그 고쳐 · 전달 중".to_string(),
            "고쳤습니다".to_string(),
            "codex · 45초 · Token -".to_string(),
        ]
    );
}

#[test]
fn initial_history_with_held_task_asks_resume_once() {
    let mut app = app();
    notify(&mut app, history(vec![task(1, 'A', TaskState::Held)]));
    let asked = matches!(app.window, Some(Window::Resume(_)));

    let effects = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    assert!(asked);
    assert_eq!(
        sent(&effects),
        vec![&Request::Continue {
            chat: ChatId(7),
            task: None
        }]
    );
    assert!(app.window.is_none());
}

#[test]
fn later_history_chunk_is_prepended_without_state() {
    let mut app = attached();
    notify(&mut app, input(2, InputState::Applied, "new"));

    notify(
        &mut app,
        history(vec![input(1, InputState::Applied, "old")]),
    );

    let first = &app.transcript.cells()[0];
    assert!(matches!(first, TranscriptCell::InputEcho { text, .. } if text == "old"));
    assert!(app.window.is_none());
}

#[test]
fn folder_trust_apply_then_confirm_sends_answer() {
    let mut app = attached();
    notify(
        &mut app,
        Notification::FolderTrustRequested {
            path: "/work/.saturn.toml".to_string(),
            fingerprint: "ab".to_string(),
            applied: Vec::new(),
            ignored: Vec::new(),
            changed_lines: Vec::new(),
        },
    );
    press(&mut app, KeyCode::Down, KeyModifiers::NONE);
    press(&mut app, KeyCode::Char('1'), KeyModifiers::NONE);

    let effects = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    assert_eq!(
        sent(&effects),
        vec![&Request::AnswerFolderTrust {
            path: "/work/.saturn.toml".to_string(),
            fingerprint: "ab".to_string(),
            apply: true,
        }]
    );
}

#[test]
fn folder_trust_q_quits() {
    let mut app = attached();
    notify(
        &mut app,
        Notification::FolderTrustRequested {
            path: "/p".to_string(),
            fingerprint: "f".to_string(),
            applied: Vec::new(),
            ignored: Vec::new(),
            changed_lines: Vec::new(),
        },
    );

    assert_eq!(
        press(&mut app, KeyCode::Char('q'), KeyModifiers::NONE),
        vec![Effect::Quit]
    );
}

#[test]
fn judge_key_prompt_sends_key_and_closes() {
    let mut app = attached();
    notify(
        &mut app,
        Notification::JudgeKeyRequired {
            reason: "invalid".to_string(),
        },
    );
    type_text(&mut app, "sk-1");

    let effects = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    assert_eq!(
        sent(&effects),
        vec![&Request::SubmitJudgeKey {
            key: "sk-1".to_string()
        }]
    );
    assert!(app.window.is_none());
    assert!(app.composer.is_empty());
}

// cost: time O(1), heap O(1), stack O(1)
// basis: estimate
#[test]
fn slash_popup_completes_command_then_shows_values() {
    let mut app = attached();

    type_text(&mut app, "/us");
    let first = app
        .popup
        .as_ref()
        .map(|p| (p.kind, p.items[0].value.clone()));
    press(&mut app, KeyCode::Tab, KeyModifiers::NONE);

    assert_eq!(first, Some((PopupKind::Command, "usage".to_string())));
    assert_eq!(app.composer.text(), "/usage ");
    assert_eq!(app.popup.as_ref().map(|p| p.kind), Some(PopupKind::Value));
    press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(app.composer.text(), "/usage chat ");
    assert!(app.popup.is_none());
}

#[test]
fn popup_escape_suppresses_until_token_changes() {
    let mut app = attached();
    type_text(&mut app, "/ta");

    press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    let closed = app.popup.is_none();
    press(&mut app, KeyCode::Char('s'), KeyModifiers::NONE);

    assert!(closed);
    assert!(app.popup.is_some());
}

#[test]
fn alt_up_recalls_latest_input() {
    let mut app = attached();
    notify(&mut app, input(1, InputState::Queued, "first"));
    notify(&mut app, input(2, InputState::Judging, "second"));

    let effects = press(&mut app, KeyCode::Up, KeyModifiers::ALT);

    assert_eq!(
        sent(&effects),
        vec![&Request::CancelInput { input: InputId(2) }]
    );
    assert_eq!(app.composer.text(), "second");
}

#[test]
fn history_keys_walk_input_history() {
    let mut app = attached();
    app.history = InputHistory::with_entries(&["one", "two"]);

    press(&mut app, KeyCode::Up, KeyModifiers::NONE);
    press(&mut app, KeyCode::Up, KeyModifiers::NONE);
    let oldest = app.composer.text();
    press(&mut app, KeyCode::Down, KeyModifiers::NONE);
    press(&mut app, KeyCode::Down, KeyModifiers::NONE);

    assert_eq!(oldest, "one");
    assert!(app.composer.is_empty());
}

#[test]
fn clicking_status_button_sends_request() {
    let mut app = attached();
    notify(&mut app, input(1, InputState::Queued, "x"));
    let now = Instant::now();
    let lines = crate::view::status_board::build(&app.chat, now);
    let areas = app.areas(app.screen, lines.len());
    let rects = crate::view::status_board::button_rects(&lines, app.lang, areas.status);
    let (rect, _) = rects[0];

    let effects = app.handle(
        AppEvent::Terminal(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: rect.x,
            row: rect.y,
            modifiers: KeyModifiers::NONE,
        })),
        now,
    );

    assert_eq!(
        sent(&effects),
        vec![&Request::SendNow { input: InputId(1) }]
    );
}

#[test]
fn close_held_button_asks_then_enter_closes() {
    let mut app = attached();
    notify(&mut app, task(5, 'E', TaskState::Held));
    app.window = None;
    app.on_button(crate::view::status_board::Button::CloseHeld(TaskId(5)));
    let area = app.key_area();

    let effects = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    assert_eq!(area, KeyArea::StatusBoard);
    assert_eq!(
        sent(&effects),
        vec![&Request::CloseHeld {
            chat: ChatId(7),
            task: TaskId(5)
        }]
    );
    assert_eq!(
        app.transcript.cells().last(),
        Some(&TranscriptCell::HeldClosed {
            label: TaskLabel('E')
        })
    );
}

#[test]
fn ctrl_t_toggles_full_transcript() {
    let mut app = attached();

    press(&mut app, KeyCode::Char('t'), KeyModifiers::CONTROL);
    let opened = matches!(app.window, Some(Window::FullTranscript(_)));
    press(&mut app, KeyCode::Char('t'), KeyModifiers::CONTROL);

    assert!(opened);
    assert!(app.window.is_none());
}

#[test]
fn question_mark_on_empty_composer_shows_shortcuts_and_any_key_closes() {
    let mut app = attached();

    press(&mut app, KeyCode::Char('?'), KeyModifiers::NONE);
    let shown = matches!(app.window, Some(Window::Shortcuts));
    press(&mut app, KeyCode::Char('x'), KeyModifiers::NONE);

    assert!(shown);
    assert!(app.window.is_none());
    assert!(app.composer.is_empty());
}

#[test]
fn render_stacks_transcript_status_composer_and_footer() {
    let mut app = attached();
    notify(&mut app, input(1, InputState::Applied, "로그인 버그 고쳐"));
    notify(&mut app, task(1, 'A', TaskState::Running));
    notify(&mut app, input(2, InputState::Queued, "테스트도"));
    let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();

    terminal
        .draw(|frame| app.render(frame, Instant::now()))
        .unwrap();

    let rows = buffer_lines(terminal.backend().buffer());
    assert!(rows[0].starts_with("> [C] 로그인 버그 고쳐 · 반영됨"));
    assert!(rows[8].starts_with("⠋ [A] 작업 중"));
    assert!(rows[9].starts_with("· [C] 대기 · 쓰기 차례 · 테스트도"));
    assert!(rows[9].ends_with("[보내기] [취소]"));
    assert_eq!(rows[10], "›");
    assert!(rows[11].starts_with("/help 도움말 · Ctrl+C 멈춤"));
    assert!(rows[11].ends_with("맥락 미확인"));
}

#[test]
fn render_single_task_hides_labels() {
    let mut app = attached();
    notify(&mut app, task(1, 'A', TaskState::Running));
    let mut terminal = Terminal::new(TestBackend::new(40, 6)).unwrap();

    terminal
        .draw(|frame| app.render(frame, Instant::now()))
        .unwrap();

    let rows = buffer_lines(terminal.backend().buffer());
    assert_eq!(rows[3], "⠋ 작업 중");
}
