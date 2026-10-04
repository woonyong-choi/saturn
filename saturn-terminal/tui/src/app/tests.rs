//! `App`의 키 처리 상태 전이, 알림 반영, 전체 화면 그리기 테스트.

use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use saturn_protocol::event::{Activity, ProviderEvent, ToolDetail, UsageReport, UsageScope};
use saturn_protocol::ids::{
    AgentId, ChatId, InputId, JudgmentId, LedgerSeq, Provider, TaskId, TaskLabel,
};
use saturn_protocol::input::{InputAnswer, InputField, InputFieldKind, InputRequest, InputValue};
use saturn_protocol::rpc::{
    Alert, ChatNotice, ExitPlan, ModelChoice, ModelInfo, ModelMode, Notification, PermissionAnswer,
    Request, UsageRange,
};
use saturn_protocol::state::{Disposition, InputState, QueueReason, TaskState};

use super::{App, AppEvent, Effect, Window};
use crate::history::InputHistory;
use crate::i18n::Lang;
use crate::keys::KeyArea;
use crate::view::buffer_lines;
use crate::view::model_picker::ModelPurpose;
use crate::view::popup::PopupKind;
use crate::view::transcript::TranscriptCell;
use crate::view::usage::usage_request;

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
        oldest: None,
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
        provider: Some(Provider::from_static("codex")),
        elapsed_ms: 45_000,
        failure: None,
    }
}

fn tool_call(task: u64, call_id: &str) -> Notification {
    Notification::TaskEvent {
        task: TaskId(task),
        event: ProviderEvent::ToolCall {
            agent: AgentId(1),
            subagent: None,
            call_id: call_id.to_string(),
            activity: Activity::ReadingFile,
            detail: ToolDetail::default(),
        },
    }
}

fn asking_to_stop(id: u64, state: InputState, text: &str) -> Notification {
    Notification::InputChanged {
        input: InputId(id),
        text: text.to_string(),
        label: Some(TaskLabel('A')),
        state,
        disposition: Some(Disposition::Queue),
        reason: (state == InputState::Queued).then_some(QueueReason::ConfirmStop),
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
            add_dirs: Vec::new(),
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
                skip_relation: false,
            }),
        ]
    );
    assert!(app.composer.is_empty());
}

#[test]
fn queue_key_while_running_queues_without_relation_judgment() {
    let mut app = attached();
    notify(&mut app, task(1, 'A', TaskState::Running));
    type_text(&mut app, "테스트도");

    let effects = press(&mut app, KeyCode::Char('q'), KeyModifiers::CONTROL);

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
fn submit_while_router_disconnected_still_sends_input() {
    let mut app = attached();
    notify(
        &mut app,
        Notification::Alert {
            alert: Alert::RouterDisconnected,
        },
    );
    type_text(&mut app, "hi");

    let effects = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    assert!(matches!(sent(&effects)[0], Request::SubmitInput { .. }));
    assert!(app.composer.is_empty());
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
    type_text(&mut app, "/usage");
    app.popup = None;

    let effects = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    assert_eq!(sent(&effects), vec![&usage_request(UsageRange::Chat)]);
    assert!(matches!(app.window, Some(Window::Usage(_))));
}

#[test]
fn usage_screen_keys_switch_range_and_second_press_returns_to_chat() {
    let mut app = attached();
    type_text(&mut app, "/usage");
    app.popup = None;
    press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    let day = press(&mut app, KeyCode::Char('d'), KeyModifiers::NONE);
    let week = press(&mut app, KeyCode::Char('w'), KeyModifiers::NONE);
    let week_again = press(&mut app, KeyCode::Char('w'), KeyModifiers::NONE);

    assert_eq!(sent(&day), vec![&usage_request(UsageRange::Day)]);
    assert_eq!(sent(&week), vec![&usage_request(UsageRange::Week)]);
    assert_eq!(sent(&week_again), vec![&usage_request(UsageRange::Chat)]);
    let Some(Window::Usage(screen)) = &app.window else {
        panic!("usage screen should stay open");
    };
    assert_eq!(screen.range, UsageRange::Chat);
}

#[test]
fn add_dir_command_sends_an_absolute_path_relative_to_the_tui_folder() {
    let mut app = attached();
    type_text(&mut app, "/add-dir ../shared lib");
    app.popup = None;

    let effects = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    assert_eq!(
        sent(&effects),
        vec![&Request::AddDir {
            chat: ChatId(7),
            path: "/work/../shared lib".to_owned(),
        }]
    );
}

#[test]
fn add_dir_command_keeps_an_absolute_path_as_it_is() {
    let mut app = attached();
    type_text(&mut app, "/add-dir /abs/dir");
    app.popup = None;

    let effects = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    assert_eq!(
        sent(&effects),
        vec![&Request::AddDir {
            chat: ChatId(7),
            path: "/abs/dir".to_owned(),
        }]
    );
}

#[test]
fn add_dir_notice_adds_a_cell_and_updates_the_start_screen_folders() {
    let mut app = attached();
    notify(
        &mut app,
        Notification::StartInfo {
            saturn_version: "0.1.0".to_string(),
            protocol_version: 1,
            providers: Vec::new(),
            router: String::new(),
            router_version: String::new(),
            folder: "/work".to_string(),
            added_dirs: Vec::new(),
        },
    );

    notify(
        &mut app,
        Notification::ChatNotice {
            chat: ChatId(7),
            task: None,
            notice: saturn_protocol::rpc::ChatNotice::FolderAdded {
                path: "/shared".to_owned(),
                applies_from_next_session: true,
            },
        },
    );

    assert!(matches!(
        app.transcript.cells().last(),
        Some(TranscriptCell::Notice { .. })
    ));
    let Some(TranscriptCell::Header(header)) = app.transcript.cells().first() else {
        panic!("the start screen should have become the header cell");
    };
    assert_eq!(header.added_dirs, vec![std::path::PathBuf::from("/shared")]);
}

#[test]
fn add_dir_notice_before_the_first_cell_updates_the_start_screen() {
    let mut app = app();
    notify(
        &mut app,
        Notification::StartInfo {
            saturn_version: "0.1.0".to_string(),
            protocol_version: 1,
            providers: Vec::new(),
            router: String::new(),
            router_version: String::new(),
            folder: "/work".to_string(),
            added_dirs: vec!["/first".to_owned()],
        },
    );
    assert_eq!(
        app.chat_folder.as_deref(),
        Some(std::path::Path::new("/work"))
    );

    notify(
        &mut app,
        Notification::ChatNotice {
            chat: ChatId(7),
            task: None,
            notice: saturn_protocol::rpc::ChatNotice::FolderAdded {
                path: "/second".to_owned(),
                applies_from_next_session: false,
            },
        },
    );

    let Some(TranscriptCell::Header(header)) = app.transcript.cells().first() else {
        panic!("the start screen should have become the header cell");
    };
    assert_eq!(
        header.added_dirs,
        vec![
            std::path::PathBuf::from("/first"),
            std::path::PathBuf::from("/second")
        ]
    );
}

#[test]
fn add_dirs_from_the_command_line_ride_on_the_attach_request() {
    let mut app = app();
    app.add_dirs = vec!["/a".to_owned(), "/b".to_owned()];

    let request = app.attach_request();

    assert!(matches!(
        request,
        Request::Attach { add_dirs, .. } if add_dirs == ["/a", "/b"]
    ));
}

#[test]
fn task_list_opened_from_a_chat_starts_in_the_chat_folder_scope() {
    let mut app = attached();
    notify(
        &mut app,
        Notification::StartInfo {
            saturn_version: "0.1.0".to_string(),
            protocol_version: 1,
            providers: Vec::new(),
            router: String::new(),
            router_version: String::new(),
            folder: "/other/project".to_string(),
            added_dirs: Vec::new(),
        },
    );
    type_text(&mut app, "/tasks");
    app.popup = None;

    press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    let Some(Window::TaskList(list)) = &app.window else {
        panic!("task list should be open");
    };
    assert_eq!(
        list.folder.as_deref(),
        Some(std::path::Path::new("/other/project"))
    );
    assert_eq!(list.scope, crate::view::task_list::FolderScope::Current);
}

// #314: 채팅 이동은 연결을 끊지 않고 붙은 채팅만 바꾼다
#[test]
fn moving_to_another_chat_only_attaches_without_detaching() {
    use crate::view::task_list::TaskListCommand;

    let mut app = attached();
    let moves = [
        (
            TaskListCommand::Open {
                chat: ChatId(8),
                task: Some(TaskId(1)),
            },
            Some(ChatId(8)),
        ),
        (TaskListCommand::NewChat, None),
    ];

    for (command, target) in moves {
        let effects = app.on_task_list_command(command);

        let requests = sent(&effects);
        assert!(!requests.contains(&&Request::Detach));
        assert_eq!(requests.len(), 1);
        assert!(matches!(requests[0], Request::Attach { chat, .. } if *chat == target));
    }
}

#[test]
fn task_list_key_a_widens_the_scope_to_all_folders() {
    let mut app = attached();
    type_text(&mut app, "/tasks");
    app.popup = None;
    press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    press(&mut app, KeyCode::Char('a'), KeyModifiers::NONE);

    let Some(Window::TaskList(list)) = &app.window else {
        panic!("task list should be open");
    };
    assert_eq!(list.scope, crate::view::task_list::FolderScope::All);
}

fn old_chat(id: u64, rows: u64) -> saturn_protocol::rpc::ChatListItem {
    saturn_protocol::rpc::ChatListItem {
        chat: ChatId(id),
        folder: "/work".to_owned(),
        name: None,
        last_active_ms: 1_700_000_000_000,
        preview: None,
        rows: Some(rows),
    }
}

fn prune_window_with(chats: Vec<saturn_protocol::rpc::ChatListItem>) -> App {
    let mut app = attached();
    type_text(&mut app, "/prune");
    app.popup = None;
    let opened = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(sent(&opened), vec![&Request::Prune { yes: false }]);
    let rows = chats.iter().filter_map(|chat| chat.rows).sum();
    notify(
        &mut app,
        Notification::PrunePreview {
            chats,
            skipped: Vec::new(),
            rows,
        },
    );
    app
}

#[test]
fn prune_window_asks_for_the_preview_and_only_y_deletes() {
    let mut app = prune_window_with(vec![old_chat(3, 40), old_chat(5, 7)]);

    let enter = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    let other = press(&mut app, KeyCode::Char('n'), KeyModifiers::NONE);
    let confirm = press(&mut app, KeyCode::Char('y'), KeyModifiers::NONE);
    let again = press(&mut app, KeyCode::Char('y'), KeyModifiers::NONE);

    assert!(sent(&enter).is_empty() && sent(&other).is_empty());
    assert_eq!(sent(&confirm), vec![&Request::Prune { yes: true }]);
    assert!(sent(&again).is_empty());
    assert!(matches!(app.window, Some(Window::Prune(_))));
}

#[test]
fn prune_window_escape_closes_without_deleting() {
    let mut app = prune_window_with(vec![old_chat(3, 40)]);

    let effects = press(&mut app, KeyCode::Esc, KeyModifiers::NONE);

    assert!(sent(&effects).is_empty());
    assert!(app.window.is_none());
}

#[test]
fn prune_window_with_nothing_to_delete_ignores_y() {
    let mut app = prune_window_with(Vec::new());

    let effects = press(&mut app, KeyCode::Char('y'), KeyModifiers::NONE);

    assert!(sent(&effects).is_empty());
}

#[test]
fn pruned_result_closes_the_window_and_leaves_one_line() {
    let mut app = prune_window_with(vec![old_chat(3, 40), old_chat(5, 7)]);
    press(&mut app, KeyCode::Char('y'), KeyModifiers::NONE);

    notify(
        &mut app,
        Notification::Pruned {
            chats: vec![old_chat(3, 40), old_chat(5, 7)],
            skipped: Vec::new(),
            rows: 47,
        },
    );

    assert!(app.window.is_none());
    assert_eq!(
        app.transcript.cells().last(),
        Some(&TranscriptCell::Warning(
            "지운 채팅 2개 · 기록 47행".to_string()
        ))
    );
}

#[test]
fn missing_retention_closes_the_prune_window_and_says_how_to_set_it() {
    let mut app = attached();
    type_text(&mut app, "/prune");
    app.popup = None;
    press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    notify(
        &mut app,
        Notification::Alert {
            alert: saturn_protocol::rpc::Alert::PruneNeedsRetention,
        },
    );

    assert!(app.window.is_none());
    assert!(matches!(
        app.transcript.cells().last(),
        Some(TranscriptCell::Warning(text)) if text.contains("retention.max_age_days")
    ));
}

#[test]
fn chat_labeled_makes_an_open_task_list_read_the_list_again() {
    let mut app = attached();
    let labeled = Notification::ChatLabeled {
        chat: ChatId(9),
        name: Some("login fix".to_owned()),
        group: None,
    };

    let closed = notify(&mut app, labeled.clone());
    type_text(&mut app, "/tasks");
    app.popup = None;
    press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    let open = notify(&mut app, labeled);

    assert!(sent(&closed).is_empty());
    assert_eq!(sent(&open), vec![&Request::ListTasks]);
}

#[test]
fn renaming_from_the_task_list_waits_for_the_labeled_notification_to_refresh() {
    use crate::view::task_list::TaskListCommand;

    let mut app = attached();

    let effects = app.on_task_list_command(TaskListCommand::Rename {
        chat: ChatId(7),
        name: "login fix".to_owned(),
    });

    assert_eq!(
        sent(&effects),
        vec![&Request::RenameChat {
            chat: ChatId(7),
            name: "login fix".to_owned(),
        }]
    );
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
fn ctrl_c_clears_draft_then_stops_then_asks_to_press_again_then_quits() {
    let mut app = attached();
    notify(&mut app, task(1, 'A', TaskState::Running));
    type_text(&mut app, "draft");

    let first = press(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
    let second = press(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
    notify(&mut app, task(1, 'A', TaskState::Held));
    let third = press(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
    let fourth = press(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
    let closed = notify(&mut app, exit_plan(ExitPlan::Close));

    assert!(first.is_empty());
    assert!(app.composer.is_empty());
    assert_eq!(sent(&second), vec![&Request::Stop { chat: ChatId(7) }]);
    assert!(sent(&third).is_empty());
    assert_eq!(
        sent(&fourth),
        vec![&Request::PrepareExit { chat: ChatId(7) }]
    );
    assert_eq!(closed, vec![Effect::Quit]);
}

fn exit_plan(plan: ExitPlan) -> Notification {
    Notification::ExitPlan { plan }
}

/// 채팅에 붙은 TUI가 빈 입력창에서 닫으려 하고 engine이 `plan`으로 답한 뒤의 화면.
fn quit_with(plan: ExitPlan) -> (App, Vec<Effect>) {
    let mut app = attached();
    press(&mut app, KeyCode::Char('d'), KeyModifiers::CONTROL);
    let effects = notify(&mut app, exit_plan(plan));
    (app, effects)
}

// #70: 닫기 전에 engine에 닫은 뒤의 처리를 묻고, 답이 오기 전에는 닫지 않는다
#[test]
fn quit_asks_the_engine_first_and_a_second_quit_closes_without_waiting() {
    let mut app = attached();

    let first = press(&mut app, KeyCode::Char('d'), KeyModifiers::CONTROL);
    let second = press(&mut app, KeyCode::Char('d'), KeyModifiers::CONTROL);

    assert_eq!(
        first,
        vec![Effect::Send(Request::PrepareExit { chat: ChatId(7) })]
    );
    assert_eq!(second, vec![Effect::Quit]);
}

// #70: background로 닫을 때 계속 실행 중인 작업 수와 다시 여는 방법을 터미널에 한 줄 남긴다
#[test]
fn exit_plan_notice_quits_and_leaves_the_running_count_line() {
    let (app, effects) = quit_with(ExitPlan::Notice { running: 2 });

    assert_eq!(effects, vec![Effect::Quit]);
    assert_eq!(
        app.exit_line().as_deref(),
        Some("작업 2개 계속 실행 중 · saturn으로 다시 여세요")
    );
}

#[test]
fn exit_plan_notice_line_is_translated() {
    let (mut app, _) = quit_with(ExitPlan::Notice { running: 1 });
    app.lang = Lang::En;

    assert_eq!(
        app.exit_line().as_deref(),
        Some("Tasks still running: 1 · Reopen with saturn")
    );
}

// #178: engine이 업데이트로 끝난다고 알리면 연결이 끊겨도 오류 없이 끝나고 다시 열라는 한 줄을 남긴다
#[test]
fn engine_restarting_alert_ends_the_tui_with_a_reopen_line() {
    let mut app = attached();
    notify(
        &mut app,
        Notification::Alert {
            alert: Alert::EngineRestarting,
        },
    );

    let effects = app.handle(AppEvent::EngineClosed, Instant::now());

    assert!(effects.is_empty());
    assert!(app.quit);
    assert!(app.restarting);
    assert_eq!(
        app.exit_line().as_deref(),
        Some("업데이트를 적용하느라 engine을 다시 시작합니다 · saturn으로 다시 여세요")
    );
}

#[test]
fn engine_restarted_alert_is_one_status_line() {
    let mut app = attached();
    notify(
        &mut app,
        Notification::Alert {
            alert: Alert::EngineRestarted,
        },
    );

    assert!(!app.restarting);
    assert_eq!(app.chat.alerts, vec![Alert::EngineRestarted]);
    assert_eq!(app.exit_line(), None);
}

#[test]
fn exit_plan_close_quits_without_a_line() {
    let (app, effects) = quit_with(ExitPlan::Close);

    assert_eq!(effects, vec![Effect::Quit]);
    assert_eq!(app.exit_line(), None);
}

#[test]
fn exit_plan_nobody_asked_for_is_ignored() {
    let mut app = attached();

    let effects = notify(&mut app, exit_plan(ExitPlan::Close));

    assert!(effects.is_empty());
}

// #70: ask는 확인 창을 띄우고, 창이 떠 있는 동안에는 닫지 않는다
#[test]
fn exit_plan_ask_opens_the_confirm_window_and_waits() {
    let (app, effects) = quit_with(ExitPlan::Ask { running: 2 });

    assert!(effects.is_empty());
    assert_eq!(app.key_area(), KeyArea::ExitConfirm);
}

#[test]
fn exit_confirm_continue_quits_and_leaves_the_running_count_line() {
    let (mut app, _) = quit_with(ExitPlan::Ask { running: 2 });

    let effects = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    assert_eq!(effects, vec![Effect::Quit]);
    assert_eq!(
        app.exit_line().as_deref(),
        Some("작업 2개 계속 실행 중 · saturn으로 다시 여세요")
    );
}

#[test]
fn exit_confirm_stop_stops_every_chat_then_quits_without_a_line() {
    let (mut app, _) = quit_with(ExitPlan::Ask { running: 2 });
    press(&mut app, KeyCode::Down, KeyModifiers::NONE);

    let effects = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    assert_eq!(effects, vec![Effect::Send(Request::StopAll), Effect::Quit]);
    assert_eq!(app.exit_line(), None);
}

// #70: Esc는 닫기를 취소하고 Ctrl+C도 작업을 멈추지 않는다
#[test]
fn exit_confirm_escape_and_ctrl_c_cancel_the_exit_without_stopping_work() {
    for (code, modifiers) in [
        (KeyCode::Esc, KeyModifiers::NONE),
        (KeyCode::Char('c'), KeyModifiers::CONTROL),
    ] {
        let (mut app, _) = quit_with(ExitPlan::Ask { running: 2 });

        let effects = press(&mut app, code, modifiers);

        assert!(effects.is_empty(), "{code:?}");
        assert_eq!(app.key_area(), KeyArea::Composer, "{code:?}");
        assert_eq!(app.exit_line(), None, "{code:?}");
    }
}

// #36: 끼워 넣기를 받지 않은 충돌 입력은 멈춤 확인 창을 띄우고, 다른 상태가 되면 지운다
#[test]
fn stop_confirm_window_opens_for_the_asking_input_and_closes_when_it_moves_on() {
    let mut app = attached();

    notify(
        &mut app,
        asking_to_stop(5, InputState::Queued, "use pytest"),
    );
    assert_eq!(app.key_area(), KeyArea::StopConfirm);
    notify(
        &mut app,
        asking_to_stop(5, InputState::Delivering, "use pytest"),
    );

    assert_eq!(app.key_area(), KeyArea::Composer);
}

// #36: 첫 선택은 작업을 멈추지 않는 대기이고, Esc도 대기다
#[test]
fn stop_confirm_enter_and_escape_answer_wait_and_down_enter_answers_stop() {
    let ask = |app: &mut App| notify(app, asking_to_stop(5, InputState::Queued, "use pytest"));
    let answer = |stop| Request::AnswerStopConfirm {
        input: InputId(5),
        stop,
    };
    let mut app = attached();
    ask(&mut app);
    let enter = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    ask(&mut app);
    let escape = press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    ask(&mut app);
    press(&mut app, KeyCode::Down, KeyModifiers::NONE);
    let down_enter = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    assert_eq!(sent(&enter), vec![&answer(false)]);
    assert_eq!(sent(&escape), vec![&answer(false)]);
    assert_eq!(sent(&down_enter), vec![&answer(true)]);
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
            provider: Provider::from_static("codex"),
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
            answer: PermissionAnswer::AllowOnce,
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
            provider: Provider::from_static("codex"),
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

fn input_requested(request_id: &str) -> Notification {
    Notification::InputRequested {
        task: TaskId(1),
        label: TaskLabel('A'),
        provider: Provider::from_static("claude"),
        request_id: request_id.to_string(),
        request: InputRequest {
            message: String::new(),
            fields: vec![InputField {
                id: "q".to_string(),
                title: "Which?".to_string(),
                description: String::new(),
                kind: InputFieldKind::Text,
                is_required: true,
                is_secret: false,
            }],
            url: None,
        },
        waiting: 0,
    }
}

#[test]
fn elicitation_window_takes_typed_text_and_sends_the_answer() {
    let mut app = attached();
    let now = Instant::now();
    app.handle(AppEvent::Engine(input_requested("r1")), now);
    let later = now + Duration::from_secs(1);

    let early = press_at(
        &mut app,
        KeyCode::Char('x'),
        now + Duration::from_millis(200),
    );
    press_at(&mut app, KeyCode::Char('o'), later);
    press_at(&mut app, KeyCode::Char('k'), later);
    let sent_answer = press_at(&mut app, KeyCode::Enter, later);

    assert!(early.is_empty());
    assert_eq!(
        sent(&sent_answer),
        vec![&Request::AnswerInput {
            request_id: "r1".to_string(),
            answer: InputAnswer::Submit {
                values: vec![("q".to_string(), InputValue::Text("ok".to_string()))],
            },
        }]
    );
    assert_eq!(app.key_area(), KeyArea::Composer);
}

#[test]
fn elicitation_window_takes_the_keyboard_and_clears_when_resolved_elsewhere() {
    let mut app = attached();
    notify(&mut app, input_requested("r1"));

    let area = app.key_area();
    notify(
        &mut app,
        Notification::InputResolved {
            request_id: "r1".to_string(),
        },
    );

    assert_eq!(area, KeyArea::Input);
    assert!(app.inputs.is_empty());
}

#[test]
fn elicitation_permission_window_comes_before_the_input_window() {
    let mut app = attached();
    notify(&mut app, input_requested("r1"));
    notify(
        &mut app,
        Notification::PermissionRequested {
            task: TaskId(1),
            label: TaskLabel('A'),
            provider: Provider::from_static("claude"),
            request_id: "p1".to_string(),
            summary: "rm".to_string(),
            reason: "clean".to_string(),
            waiting: 0,
        },
    );

    assert_eq!(app.key_area(), KeyArea::Permission);
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
    assert!(has_correction(&app));
}

fn has_correction(app: &App) -> bool {
    app.transcript
        .cells()
        .iter()
        .any(|cell| matches!(cell, TranscriptCell::Correction { .. }))
}

fn has_feedback_question(app: &App) -> bool {
    app.transcript
        .cells()
        .iter()
        .any(|cell| matches!(cell, TranscriptCell::Feedback { .. }))
}

/// 입력 1이 아직 보내지 않은 상태에서 틀림으로 답해 바로잡기 제안이 뜬 앱.
fn with_correction() -> App {
    let mut app = attached();
    notify(&mut app, input(1, InputState::Queued, "x"));
    feedback(&mut app, Instant::now());
    press(&mut app, KeyCode::Char('2'), KeyModifiers::NONE);
    assert!(has_correction(&app));
    app
}

#[test]
fn correction_enter_runs_the_first_choice_as_a_new_task() {
    let mut app = with_correction();

    let effects = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    assert_eq!(
        sent(&effects),
        vec![&Request::RunAsNewTask { input: InputId(1) }]
    );
    assert!(!has_correction(&app));
}

#[test]
fn correction_down_then_enter_keeps_the_input_as_it_is() {
    let mut app = with_correction();

    press(&mut app, KeyCode::Down, KeyModifiers::NONE);
    let effects = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    assert!(sent(&effects).is_empty());
    assert!(!has_correction(&app));
}

#[test]
fn correction_up_and_down_move_between_the_two_choices() {
    let mut app = with_correction();

    press(&mut app, KeyCode::Down, KeyModifiers::NONE);
    press(&mut app, KeyCode::Up, KeyModifiers::NONE);
    let effects = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    assert_eq!(
        sent(&effects),
        vec![&Request::RunAsNewTask { input: InputId(1) }]
    );
}

#[test]
fn correction_digits_pick_a_choice_while_the_composer_is_empty() {
    let mut run = with_correction();
    let mut keep = with_correction();

    let run_effects = press(&mut run, KeyCode::Char('1'), KeyModifiers::NONE);
    let keep_effects = press(&mut keep, KeyCode::Char('2'), KeyModifiers::NONE);

    assert_eq!(
        sent(&run_effects),
        vec![&Request::RunAsNewTask { input: InputId(1) }]
    );
    assert!(sent(&keep_effects).is_empty());
    assert!(!has_correction(&keep));
    assert!(run.composer.is_empty() && keep.composer.is_empty());
}

#[test]
fn correction_takes_the_arrow_keys_until_escape_gives_them_back_to_history() {
    let mut app = with_correction();
    app.history = InputHistory::with_entries(&["one", "two"]);

    press(&mut app, KeyCode::Up, KeyModifiers::NONE);
    let while_open = app.composer.text();
    let effects = press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    press(&mut app, KeyCode::Up, KeyModifiers::NONE);

    assert_eq!(while_open, "");
    assert!(sent(&effects).is_empty());
    assert!(!has_correction(&app));
    assert_eq!(app.composer.text(), "two");
}

#[test]
fn correction_closed_with_escape_opens_again_with_the_feedback_command() {
    let mut app = with_correction();
    press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    type_text(&mut app, "/feedback");
    app.popup = None;

    let effects = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    let reopened = has_correction(&app);
    let run = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    assert!(sent(&effects).is_empty());
    assert!(reopened);
    assert_eq!(
        sent(&run),
        vec![&Request::RunAsNewTask { input: InputId(1) }]
    );
}

#[test]
fn correction_goes_away_when_the_input_is_no_longer_unsent() {
    let mut app = with_correction();

    notify(&mut app, input(1, InputState::Delivering, "x"));

    assert!(!has_correction(&app));
    let effects = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert!(sent(&effects).is_empty());
}

#[test]
fn digits_after_a_draft_start_do_not_answer_the_correction() {
    let mut app = with_correction();

    type_text(&mut app, "v1");

    assert_eq!(app.composer.text(), "v1");
    assert!(has_correction(&app));
}

#[test]
fn digits_after_a_draft_start_do_not_answer_the_feedback_question() {
    let mut app = attached();
    feedback(&mut app, Instant::now());

    type_text(&mut app, "v2");
    let area = app.key_area();

    assert_eq!(app.composer.text(), "v2");
    assert_eq!(area, KeyArea::Composer);
    assert!(app.chat.feedback.is_some());
    assert!(has_feedback_question(&app));
}

#[test]
fn a_number_draft_is_not_taken_as_a_feedback_answer() {
    let mut app = attached();
    app.set_draft("12");
    feedback(&mut app, Instant::now());

    let effects = press(&mut app, KeyCode::Char('1'), KeyModifiers::NONE);

    assert!(sent(&effects).is_empty());
    assert_eq!(app.composer.text(), "121");
    assert!(app.chat.feedback.is_some());
}

#[test]
fn digits_go_to_the_composer_when_no_choice_is_shown() {
    let mut app = attached();

    press(&mut app, KeyCode::Char('1'), KeyModifiers::NONE);

    assert_eq!(app.composer.text(), "1");
}

#[test]
fn feedback_arrows_and_enter_pick_an_answer() {
    let mut app = attached();
    feedback(&mut app, Instant::now());

    press(&mut app, KeyCode::Down, KeyModifiers::NONE);
    let effects = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    assert_eq!(
        sent(&effects),
        vec![&Request::AnswerFeedback {
            judgment: JudgmentId(9),
            correct: false,
        }]
    );
    assert!(app.chat.feedback.is_none());
}

#[test]
fn feedback_escape_closes_without_a_request_and_arrows_return_to_history() {
    let mut app = attached();
    app.history = InputHistory::with_entries(&["one", "two"]);
    feedback(&mut app, Instant::now());

    press(&mut app, KeyCode::Up, KeyModifiers::NONE);
    let while_open = app.composer.text();
    let effects = press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    press(&mut app, KeyCode::Up, KeyModifiers::NONE);

    assert_eq!(while_open, "");
    assert!(sent(&effects).is_empty());
    assert!(app.chat.feedback.is_none());
    assert!(!has_feedback_question(&app));
    assert_eq!(app.composer.text(), "two");
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
            protocol_version: 1,
            providers: Vec::new(),
            router: String::new(),
            router_version: String::new(),
            folder: "/work".to_string(),
            added_dirs: Vec::new(),
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

fn reply_piece(task: u64, text: &str) -> Notification {
    Notification::TaskEvent {
        task: TaskId(task),
        event: ProviderEvent::Text {
            agent: AgentId(1),
            subagent: None,
            text: text.to_string(),
        },
    }
}

#[test]
fn first_history_chunk_shows_earlier_input_and_finished_reply() {
    let mut app = app();

    notify(
        &mut app,
        history(vec![
            input(1, InputState::Applied, "old question"),
            task(1, 'A', TaskState::Running),
            reply_piece(1, "old "),
            reply_piece(1, "answer"),
            task(1, 'A', TaskState::Done),
        ]),
    );

    let cells = app.transcript.cells();
    assert!(cells.iter().any(|cell| matches!(
        cell,
        TranscriptCell::InputEcho { text, .. } if text == "old question"
    )));
    assert!(cells.iter().any(|cell| matches!(
        cell,
        TranscriptCell::AgentText { lines, .. } if lines.join("") == "old answer"
    )));
}

fn history_page(oldest: Option<u64>, has_more: bool) -> Notification {
    Notification::HistoryChunk {
        chat: ChatId(7),
        entries: vec![input(1, InputState::Applied, "old question")],
        oldest: oldest.map(LedgerSeq),
        has_more,
    }
}

fn wheel_up(app: &mut App) -> Vec<Effect> {
    app.handle(
        AppEvent::Terminal(Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 1,
            row: 1,
            modifiers: KeyModifiers::NONE,
        })),
        Instant::now(),
    )
}

fn load_history(before: u64) -> Request {
    Request::LoadHistory {
        chat: ChatId(7),
        before: Some(LedgerSeq(before)),
        limit: super::HISTORY_PAGE,
    }
}

#[test]
fn scrolling_to_the_top_asks_for_history_before_the_oldest_received() {
    let mut app = app();
    notify(&mut app, history_page(Some(400), true));

    let first = wheel_up(&mut app);
    notify(&mut app, history_page(Some(150), true));
    let second = wheel_up(&mut app);

    assert_eq!(sent(&first), vec![&load_history(400)]);
    assert_eq!(sent(&second), vec![&load_history(150)]);
}

#[test]
fn scrolling_stops_asking_once_the_start_of_the_chat_is_reached() {
    let mut app = app();
    notify(&mut app, history_page(Some(400), true));
    wheel_up(&mut app);
    notify(&mut app, history_page(Some(150), false));

    let effects = wheel_up(&mut app);

    assert!(sent(&effects).is_empty());
}

#[test]
fn scrolling_without_a_known_position_does_not_ask() {
    let mut app = app();
    notify(&mut app, history_page(None, true));

    let effects = wheel_up(&mut app);

    assert!(sent(&effects).is_empty());
}

fn usage_event(task: u64, input: u64, output: u64) -> Notification {
    Notification::TaskEvent {
        task: TaskId(task),
        event: ProviderEvent::Usage(UsageReport {
            agent: AgentId(1),
            subagent: None,
            model: Some("gpt-x".to_owned()),
            scope: UsageScope::MainTurn,
            input: Some(input),
            cache_read: None,
            cache_write: None,
            output: Some(output),
            reasoning: None,
        }),
    }
}

/// 다시 열 때 실행 줄이 실시간처럼 토큰 수를 보인다(#384).
#[test]
fn reattached_result_line_shows_the_usage_of_the_run() {
    let mut app = app();

    notify(
        &mut app,
        history(vec![
            input(1, InputState::Applied, "question"),
            task(1, 'A', TaskState::Running),
            reply_piece(1, "answer"),
            usage_event(1, 100, 20),
            task(1, 'A', TaskState::Done),
        ]),
    );

    assert!(app.transcript.cells().iter().any(|cell| matches!(
        cell,
        TranscriptCell::Result {
            tokens: Some(120),
            ..
        }
    )));
}

/// 다시 열 때 provider 전환 줄이 실시간처럼 나온다(#384).
#[test]
fn reattached_chat_shows_the_provider_switch_line() {
    let mut app = app();

    notify(
        &mut app,
        history(vec![
            input(1, InputState::Applied, "question"),
            Notification::ChatNotice {
                chat: ChatId(7),
                task: None,
                notice: ChatNotice::ProviderSwitched {
                    from: Provider::from_static("codex"),
                    to: Provider::from_static("claude"),
                },
            },
            task(1, 'A', TaskState::Running),
            task(1, 'A', TaskState::Done),
        ]),
    );

    assert!(app.transcript.cells().iter().any(|cell| matches!(
        cell,
        TranscriptCell::Notice {
            notice: ChatNotice::ProviderSwitched { from, to },
            ..
        } if from.as_str() == "codex" && to.as_str() == "claude"
    )));
}

/// 다시 열 때 고정 모델 안내 줄이 실시간처럼 나온다(#384).
#[test]
fn reattached_chat_shows_the_pinned_model_line() {
    let mut app = app();
    notify(&mut app, history(vec![input(1, InputState::Applied, "q")]));

    notify(
        &mut app,
        Notification::ModelPinned {
            chat: ChatId(7),
            model: ModelChoice {
                provider: Provider::from_static("codex"),
                model: "gpt-x".to_owned(),
            },
        },
    );

    assert!(app.transcript.cells().iter().any(|cell| matches!(
        cell,
        TranscriptCell::Warning(text) if text.contains("codex") && text.contains("gpt-x")
    )));
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

    let asked = press(&mut app, KeyCode::Char('q'), KeyModifiers::NONE);
    let closed = notify(&mut app, exit_plan(ExitPlan::Close));

    assert_eq!(
        sent(&asked),
        vec![&Request::PrepareExit { chat: ChatId(7) }]
    );
    assert_eq!(closed, vec![Effect::Quit]);
}

#[test]
fn router_key_prompt_sends_key_and_closes() {
    let mut app = attached();
    notify(
        &mut app,
        Notification::RouterKeyRequired {
            reason: "invalid".to_string(),
        },
    );
    type_text(&mut app, "sk-1");

    let effects = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    assert_eq!(
        sent(&effects),
        vec![&Request::SubmitRouterKey {
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

    type_text(&mut app, "/rec");
    let first = app
        .popup
        .as_ref()
        .map(|p| (p.kind, p.items[0].value.clone()));
    press(&mut app, KeyCode::Tab, KeyModifiers::NONE);

    assert_eq!(first, Some((PopupKind::Command, "record".to_string())));
    assert_eq!(app.composer.text(), "/record ");
    assert_eq!(app.popup.as_ref().map(|p| p.kind), Some(PopupKind::Value));
    press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(app.composer.text(), "/record on ");
    assert!(app.popup.is_none());
}

#[test]
fn commands_notification_fills_the_slash_and_dollar_popups() {
    let mut app = attached();
    notify(
        &mut app,
        Notification::Commands {
            provider: Provider::from_static("fake-agent"),
            commands: vec![
                saturn_protocol::rpc::CommandInfo {
                    name: "zz-review".to_owned(),
                    description: "review".to_owned(),
                    is_skill: false,
                },
                saturn_protocol::rpc::CommandInfo {
                    name: "zz-deploy".to_owned(),
                    description: "deploy".to_owned(),
                    is_skill: true,
                },
            ],
        },
    );

    type_text(&mut app, "/zz");
    let command = app.popup.as_ref().map(|p| p.items[0].value.clone());
    app.popup = None;
    app.composer.clear();
    type_text(&mut app, "$zz");
    let skill = app.popup.as_ref().map(|p| p.items[0].value.clone());

    assert_eq!(command.as_deref(), Some("zz-review"));
    assert_eq!(skill.as_deref(), Some("$zz-deploy"));
}

#[test]
fn model_command_values_are_the_provider_ids_engine_announced() {
    let mut app = attached();
    app.start = Some(crate::view::start_screen::StartInfo {
        saturn_version: "0.1.0".to_string(),
        providers: ["alpha-agent", "beta-agent"]
            .into_iter()
            .map(|id| crate::view::start_screen::StartProvider {
                provider: Provider::from_static(id),
                display_name: id.to_uppercase(),
                version: None,
            })
            .collect(),
        router: None,
        router_version: None,
        folder: std::path::PathBuf::from("/w"),
        added_dirs: Vec::new(),
    });

    type_text(&mut app, "/model ");

    let values: Vec<String> = app
        .popup
        .as_ref()
        .map(|p| p.items.iter().map(|item| item.value.clone()).collect())
        .unwrap_or_default();
    assert_eq!(values, ["alpha-agent", "beta-agent"]);
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

fn model_info(provider: Provider, model: &str) -> ModelInfo {
    ModelInfo {
        choice: ModelChoice {
            provider,
            model: model.to_owned(),
        },
        name: model.to_owned(),
    }
}

/// `/model`을 실행하고 목록 두 개가 도착한 상태.
fn model_window() -> App {
    let mut app = attached();
    type_text(&mut app, "/model");
    app.popup = None;
    press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    notify(
        &mut app,
        Notification::Models {
            models: vec![
                model_info(Provider::from_static("claude"), "opus"),
                model_info(Provider::from_static("codex"), "gpt-x"),
            ],
        },
    );
    app
}

#[test]
fn model_command_asks_for_the_list_and_opens_the_window() {
    let mut app = attached();
    type_text(&mut app, "/model");
    app.popup = None;

    let effects = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    assert_eq!(
        sent(&effects),
        vec![&Request::ListModels {
            chat: ChatId(7),
            provider: None,
        }]
    );
    assert!(matches!(app.window, Some(Window::Model(_))));
}

#[test]
fn model_command_with_a_provider_asks_only_for_that_provider() {
    let mut app = attached();
    type_text(&mut app, "/model codex");
    app.popup = None;

    let effects = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    assert_eq!(
        sent(&effects),
        vec![&Request::ListModels {
            chat: ChatId(7),
            provider: Some(Provider::from_static("codex")),
        }]
    );
}

#[test]
fn model_window_enter_asks_the_engine_to_pin_the_model() {
    let mut app = model_window();

    press(&mut app, KeyCode::Down, KeyModifiers::NONE);
    let effects = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    assert!(app.window.is_none());
    assert_eq!(
        sent(&effects),
        vec![&Request::SetModel {
            chat: ChatId(7),
            model: ModelChoice {
                provider: Provider::from_static("codex"),
                model: "gpt-x".to_owned(),
            },
        }]
    );
}

#[test]
fn model_window_escape_sends_nothing() {
    let mut app = model_window();

    press(&mut app, KeyCode::Down, KeyModifiers::NONE);
    let effects = press(&mut app, KeyCode::Esc, KeyModifiers::NONE);

    assert!(app.window.is_none());
    assert!(sent(&effects).is_empty());
}

#[test]
fn pinned_model_notice_marks_the_model_in_the_next_window() {
    let mut app = attached();
    let model = ModelChoice {
        provider: Provider::from_static("claude"),
        model: "opus".to_owned(),
    };
    notify(
        &mut app,
        Notification::ModelPinned {
            chat: ChatId(7),
            model: model.clone(),
        },
    );

    type_text(&mut app, "/model");
    app.popup = None;
    press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    let Some(Window::Model(picker)) = &app.window else {
        panic!("model window should be open");
    };
    assert_eq!(picker.current, Some(model));
}

#[test]
fn model_command_is_not_passed_to_the_provider() {
    let mut app = attached();
    type_text(&mut app, "/model");
    app.popup = None;

    let effects = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    assert!(
        sent(&effects)
            .iter()
            .all(|request| !matches!(request, Request::SubmitInput { .. }))
    );
}

fn model_settings(default: Option<ModelChoice>, mode: ModelMode) -> Notification {
    Notification::ModelSettings {
        chat: ChatId(7),
        default,
        mode,
    }
}

fn opus() -> ModelChoice {
    ModelChoice {
        provider: Provider::from_static("claude"),
        model: "opus".to_owned(),
    }
}

/// 기본 모델이 없다는 알림을 받아 처음 고르기 창이 열리고 목록 두 개가 도착한 상태.
fn first_default_window() -> App {
    let mut app = attached();
    notify(&mut app, model_settings(None, ModelMode::Auto));
    notify(
        &mut app,
        Notification::Models {
            models: vec![
                model_info(Provider::from_static("claude"), "opus"),
                model_info(Provider::from_static("codex"), "gpt-x"),
            ],
        },
    );
    app
}

#[test]
fn missing_default_model_opens_the_first_choice_window_with_every_provider() {
    let mut app = attached();

    let effects = notify(&mut app, model_settings(None, ModelMode::Auto));

    let Some(Window::Model(picker)) = &app.window else {
        panic!("first choice window should be open");
    };
    assert_eq!(picker.purpose, ModelPurpose::Default);
    assert_eq!(
        sent(&effects),
        vec![&Request::ListModels {
            chat: ChatId(7),
            provider: None,
        }]
    );
}

#[test]
fn chosen_default_model_does_not_open_the_first_choice_window() {
    let mut app = attached();

    let effects = notify(&mut app, model_settings(Some(opus()), ModelMode::Auto));

    assert!(app.window.is_none());
    assert!(sent(&effects).is_empty());
}

#[test]
fn first_choice_window_enter_saves_the_default_model() {
    let mut app = first_default_window();

    press(&mut app, KeyCode::Down, KeyModifiers::NONE);
    let effects = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    assert!(app.window.is_none());
    assert_eq!(
        sent(&effects),
        vec![&Request::SetDefaultModel {
            chat: ChatId(7),
            model: ModelChoice {
                provider: Provider::from_static("codex"),
                model: "gpt-x".to_owned(),
            },
        }]
    );
}

#[test]
fn first_choice_window_escape_saves_nothing_and_is_not_reopened_in_this_run() {
    let mut app = first_default_window();

    let effects = press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    let again = notify(&mut app, model_settings(None, ModelMode::Auto));

    assert!(sent(&effects).is_empty());
    assert!(sent(&again).is_empty());
    assert!(app.window.is_none());
}

#[test]
fn first_choice_window_ignores_the_mode_key() {
    let mut app = first_default_window();

    let effects = press(&mut app, KeyCode::Char('m'), KeyModifiers::NONE);

    assert!(sent(&effects).is_empty());
}

#[test]
fn model_window_d_saves_the_highlighted_model_as_the_default() {
    let mut app = model_window();

    press(&mut app, KeyCode::Down, KeyModifiers::NONE);
    let effects = press(&mut app, KeyCode::Char('d'), KeyModifiers::NONE);

    assert!(matches!(app.window, Some(Window::Model(_))));
    assert_eq!(
        sent(&effects),
        vec![&Request::SetDefaultModel {
            chat: ChatId(7),
            model: ModelChoice {
                provider: Provider::from_static("codex"),
                model: "gpt-x".to_owned(),
            },
        }]
    );
}

#[test]
fn model_window_m_switches_between_auto_and_manual() {
    let mut app = attached();
    notify(&mut app, model_settings(Some(opus()), ModelMode::Auto));
    type_text(&mut app, "/model");
    app.popup = None;
    press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    let to_manual = press(&mut app, KeyCode::Char('m'), KeyModifiers::NONE);
    notify(&mut app, model_settings(Some(opus()), ModelMode::Manual));
    let to_auto = press(&mut app, KeyCode::Char('m'), KeyModifiers::NONE);

    assert_eq!(
        sent(&to_manual),
        vec![&Request::SetModelMode {
            chat: ChatId(7),
            mode: ModelMode::Manual,
        }]
    );
    assert_eq!(
        sent(&to_auto),
        vec![&Request::SetModelMode {
            chat: ChatId(7),
            mode: ModelMode::Auto,
        }]
    );
}

#[test]
fn model_settings_change_leaves_a_notice_and_updates_the_footer_state() {
    let mut app = attached();
    notify(&mut app, model_settings(Some(opus()), ModelMode::Auto));

    notify(&mut app, model_settings(Some(opus()), ModelMode::Manual));

    assert_eq!(app.chat.model_mode, Some(ModelMode::Manual));
    assert!(app.transcript.cells().iter().any(|cell| matches!(
        cell,
        TranscriptCell::Warning(text) if text.contains("매뉴얼")
    )));
}

#[test]
fn interrupted_tool_is_marked_when_the_task_is_held_without_a_result() {
    let mut app = attached();
    notify(&mut app, task(1, 'A', TaskState::Running));
    notify(&mut app, tool_call(1, "c1"));

    notify(&mut app, task(1, 'A', TaskState::Held));

    let marked = app.transcript.cells().iter().any(|cell| {
        matches!(
            cell,
            TranscriptCell::Tool {
                is_interrupted: true,
                ..
            }
        )
    });
    assert!(marked);
}

fn queued_and_held() -> App {
    let mut app = attached();
    notify(&mut app, input(1, InputState::Queued, "x"));
    notify(&mut app, task(5, 'E', TaskState::Held));
    app
}

fn settings_applied(keymap: Option<&str>) -> Notification {
    Notification::SettingsApplied {
        revision: saturn_protocol::ids::SettingsRevision(2),
        warning: None,
        keymap: keymap.map(str::to_owned),
    }
}

fn last_warning(app: &App) -> String {
    match app.transcript.cells().last() {
        Some(TranscriptCell::Warning(text)) => text.clone(),
        other => panic!("expected a warning line, got {other:?}"),
    }
}

fn mode_of(effects: &[Effect]) -> String {
    match sent(effects)[..] {
        [Request::SetPermissionMode { mode, .. }] => mode.clone(),
        _ => String::new(),
    }
}

fn run_command_line(app: &mut App, line: &str) -> Vec<Effect> {
    type_text(app, line);
    app.popup = None;
    press(app, KeyCode::Enter, KeyModifiers::NONE)
}

#[test]
fn escape_stops_running_work_and_does_nothing_when_idle() {
    let mut idle_app = attached();
    let mut app = attached();
    notify(&mut app, task(1, 'A', TaskState::Running));

    let idle = press(&mut idle_app, KeyCode::Esc, KeyModifiers::NONE);
    let running = press(&mut app, KeyCode::Esc, KeyModifiers::NONE);

    assert!(idle.is_empty());
    assert_eq!(sent(&running), vec![&Request::Stop { chat: ChatId(7) }]);
}

#[test]
fn escape_closes_an_open_popup_before_anything_else() {
    let mut app = attached();
    notify(&mut app, task(1, 'A', TaskState::Running));
    type_text(&mut app, "/rec");
    assert!(app.popup.is_some());

    let effects = press(&mut app, KeyCode::Esc, KeyModifiers::NONE);

    assert!(app.popup.is_none());
    assert!(sent(&effects).is_empty());
}

#[test]
fn double_escape_says_rewind_is_not_implemented_and_the_command_does_too() {
    let mut app = attached();
    press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    let by_keys = last_warning(&app);

    run_command_line(&mut app, "/rewind");

    assert_eq!(by_keys, "되돌리기는 아직 구현되지 않았습니다");
    assert_eq!(last_warning(&app), by_keys);
}

#[test]
fn shift_tab_cycles_the_permission_mode_from_the_default() {
    let mut app = attached();

    let first = press(&mut app, KeyCode::BackTab, KeyModifiers::SHIFT);
    let second = press(&mut app, KeyCode::BackTab, KeyModifiers::SHIFT);
    let third = press(&mut app, KeyCode::BackTab, KeyModifiers::SHIFT);

    assert_eq!(mode_of(&first), "read-only");
    assert_eq!(mode_of(&second), "ask");
    assert_eq!(mode_of(&third), "edit");
    assert_eq!(last_warning(&app), "권한 모드: edit");
    assert!(app.composer.is_empty());
}

#[test]
fn mode_command_cycles_without_a_value_and_sets_with_one() {
    let mut app = attached();

    let cycled = run_command_line(&mut app, "/mode");
    let set = run_command_line(&mut app, "/mode full");
    let after_full = press(&mut app, KeyCode::BackTab, KeyModifiers::SHIFT);

    assert_eq!(mode_of(&cycled), "read-only");
    assert_eq!(mode_of(&set), "full");
    assert_eq!(mode_of(&after_full), "ask");
}

#[test]
fn permissions_command_sets_where_shift_tab_continues() {
    let mut app = attached();
    run_command_line(&mut app, "/permissions read-only");

    let effects = press(&mut app, KeyCode::BackTab, KeyModifiers::SHIFT);

    assert_eq!(mode_of(&effects), "ask");
}

#[test]
fn tab_completes_the_command_list_and_no_longer_submits() {
    let mut app = attached();
    type_text(&mut app, "hello");
    let submit = press(&mut app, KeyCode::Tab, KeyModifiers::NONE);
    app.composer.clear();
    type_text(&mut app, "/ta");
    press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    let closed = app.popup.is_none();

    press(&mut app, KeyCode::Tab, KeyModifiers::NONE);

    assert!(sent(&submit).is_empty());
    assert_eq!(app.composer.text(), "/ta");
    assert!(closed);
    assert!(app.popup.is_some());
}

#[test]
fn left_arrow_on_an_empty_composer_does_not_open_the_task_list() {
    let mut app = attached();

    press(&mut app, KeyCode::Left, KeyModifiers::NONE);

    assert!(app.window.is_none());
}

#[test]
fn f5_and_the_tasks_command_open_the_task_list() {
    let mut by_key = attached();
    let mut by_command = attached();

    let key_effects = press(&mut by_key, KeyCode::F(5), KeyModifiers::NONE);
    let command_effects = run_command_line(&mut by_command, "/tasks");

    assert!(matches!(by_key.window, Some(Window::TaskList(_))));
    assert_eq!(sent(&key_effects), vec![&Request::ListTasks]);
    assert_eq!(sent(&command_effects), vec![&Request::ListTasks]);
}

#[test]
fn f3_and_the_agents_command_enter_the_status_board_and_enter_runs_the_button() {
    let mut by_key = queued_and_held();
    let mut by_command = queued_and_held();

    press(&mut by_key, KeyCode::F(3), KeyModifiers::NONE);
    run_command_line(&mut by_command, "/agents");
    let area = by_key.key_area();
    let effects = press(&mut by_key, KeyCode::Enter, KeyModifiers::NONE);

    assert_eq!(area, KeyArea::BoardFocus);
    assert_eq!(by_command.key_area(), KeyArea::BoardFocus);
    assert_eq!(
        sent(&effects),
        vec![&Request::SendNow { input: InputId(1) }]
    );
    assert_eq!(by_key.key_area(), KeyArea::Composer);
}

#[test]
fn status_board_focus_moves_between_buttons_and_lines() {
    let mut app = queued_and_held();
    press(&mut app, KeyCode::F(3), KeyModifiers::NONE);

    press(&mut app, KeyCode::Right, KeyModifiers::NONE);
    let cancel = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    press(&mut app, KeyCode::F(3), KeyModifiers::NONE);
    press(&mut app, KeyCode::Down, KeyModifiers::NONE);
    let held = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    assert_eq!(
        sent(&cancel),
        vec![&Request::CancelInput { input: InputId(1) }]
    );
    assert_eq!(
        sent(&held),
        vec![&Request::Continue {
            chat: ChatId(7),
            task: Some(TaskId(5))
        }]
    );
}

#[test]
fn status_board_focus_takes_arrows_until_escape_returns_them_to_history() {
    let mut app = queued_and_held();
    app.history = InputHistory::with_entries(&["one", "two"]);
    press(&mut app, KeyCode::F(3), KeyModifiers::NONE);

    press(&mut app, KeyCode::Up, KeyModifiers::NONE);
    let while_focused = app.composer.text();
    let effects = press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    press(&mut app, KeyCode::Up, KeyModifiers::NONE);

    assert_eq!(while_focused, "");
    assert!(sent(&effects).is_empty());
    assert_eq!(app.composer.text(), "two");
}

#[test]
fn status_board_focus_does_not_start_without_buttons_and_ends_when_the_line_goes() {
    let mut app = attached();
    press(&mut app, KeyCode::F(3), KeyModifiers::NONE);
    let without = app.key_area();
    notify(&mut app, input(1, InputState::Queued, "x"));
    press(&mut app, KeyCode::F(3), KeyModifiers::NONE);

    notify(&mut app, input(1, InputState::Cancelled, "x"));
    let effects = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    assert_eq!(without, KeyArea::Composer);
    assert!(sent(&effects).is_empty());
    assert_eq!(app.key_area(), KeyArea::Composer);
}

#[test]
fn ctrl_c_twice_on_an_idle_empty_composer_quits_and_another_key_in_between_resets() {
    let mut app = attached();
    let first = press(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
    let hint = last_warning(&app);
    type_text(&mut app, "x");
    app.composer.clear();
    let after_typing = press(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
    let second = press(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);

    assert!(sent(&first).is_empty());
    assert_eq!(hint, "한 번 더 누르면 종료합니다");
    assert!(sent(&after_typing).is_empty());
    assert_eq!(
        sent(&second),
        vec![&Request::PrepareExit { chat: ChatId(7) }]
    );
}

#[test]
fn line_editing_keys_move_and_delete_inside_the_draft() {
    let mut app = attached();
    type_text(&mut app, "one two three");

    press(&mut app, KeyCode::Char('w'), KeyModifiers::CONTROL);
    let after_word = app.composer.text();
    press(&mut app, KeyCode::Char('a'), KeyModifiers::CONTROL);
    type_text(&mut app, ">");
    press(&mut app, KeyCode::Char('e'), KeyModifiers::CONTROL);
    type_text(&mut app, "<");
    let marked = app.composer.text();
    press(&mut app, KeyCode::Char('u'), KeyModifiers::CONTROL);

    assert_eq!(after_word, "one two ");
    assert_eq!(marked, ">one two <");
    assert_eq!(app.composer.text(), "");
}

#[test]
fn ctrl_l_redraws_the_screen_and_keeps_the_draft() {
    let mut app = attached();
    type_text(&mut app, "draft");

    let effects = press(&mut app, KeyCode::Char('l'), KeyModifiers::CONTROL);

    assert!(effects.contains(&Effect::Redraw));
    assert_eq!(app.composer.text(), "draft");
}

#[test]
fn keymap_command_switches_the_preset_and_changes_escape_tab_and_ctrl_c() {
    let mut app = attached();
    notify(&mut app, task(1, 'A', TaskState::Running));
    run_command_line(&mut app, "/keymap gemini");
    let announced = last_warning(&app);

    let escape = press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    type_text(&mut app, "later");
    let tab = press(&mut app, KeyCode::Tab, KeyModifiers::NONE);

    assert_eq!(announced, "키 묶음: gemini");
    assert!(sent(&escape).is_empty());
    assert!(matches!(
        sent(&tab)[0],
        Request::SubmitInput {
            skip_relation: true,
            ..
        }
    ));
}

#[test]
fn keymap_command_without_a_name_lists_the_presets_and_a_bad_name_is_a_warning() {
    let mut app = attached();

    run_command_line(&mut app, "/keymap");
    let listed = last_warning(&app);
    run_command_line(&mut app, "/keymap vim");

    assert_eq!(
        listed,
        "키 묶음: saturn (saturn, claude, codex, gemini, opencode)"
    );
    assert!(last_warning(&app).contains("/keymap vim"));
    assert_eq!(app.keymap.name(), "saturn");
}

#[test]
fn opencode_preset_turns_tab_into_the_permission_mode_cycle() {
    let mut app = attached();
    run_command_line(&mut app, "/keymap opencode");

    let tab = press(&mut app, KeyCode::Tab, KeyModifiers::NONE);

    assert_eq!(mode_of(&tab), "read-only");
}

#[test]
fn the_setting_picks_the_preset_and_a_repeated_value_keeps_the_keymap_command_choice() {
    let mut app = attached();
    notify(&mut app, settings_applied(Some("claude")));
    let from_setting = app.keymap.name();
    run_command_line(&mut app, "/keymap gemini");

    notify(&mut app, settings_applied(Some("claude")));
    let kept = app.keymap.name();
    notify(&mut app, settings_applied(None));
    let kept_without_value = app.keymap.name();
    notify(&mut app, settings_applied(Some("opencode")));

    assert_eq!(from_setting, "claude");
    assert_eq!(kept, "gemini");
    assert_eq!(kept_without_value, "gemini");
    assert_eq!(app.keymap.name(), "opencode");
}

#[test]
fn claude_preset_moves_the_full_transcript_to_ctrl_o() {
    let mut app = attached();
    run_command_line(&mut app, "/keymap claude");

    press(&mut app, KeyCode::Char('t'), KeyModifiers::CONTROL);
    let ctrl_t = app.window.is_none();
    press(&mut app, KeyCode::Char('o'), KeyModifiers::CONTROL);

    assert!(ctrl_t);
    assert!(matches!(app.window, Some(Window::FullTranscript(_))));
}

#[test]
fn stop_transcript_redraw_and_quit_commands_do_what_their_keys_do() {
    let mut app = attached();
    notify(&mut app, task(1, 'A', TaskState::Running));

    let stop = run_command_line(&mut app, "/stop");
    let redraw = run_command_line(&mut app, "/redraw");
    run_command_line(&mut app, "/transcript");
    let transcript = matches!(app.window, Some(Window::FullTranscript(_)));
    app.window = None;
    let quit = run_command_line(&mut app, "/quit");

    assert_eq!(sent(&stop), vec![&Request::Stop { chat: ChatId(7) }]);
    assert!(redraw.contains(&Effect::Redraw));
    assert!(transcript);
    assert_eq!(sent(&quit), vec![&Request::PrepareExit { chat: ChatId(7) }]);
}
