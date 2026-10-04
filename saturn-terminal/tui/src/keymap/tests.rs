//! 해석기와 표 검사. 모든 프리셋이 같은 불변식을 지키는지 강제한다.

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use saturn_protocol::keymap::PRESET_NAMES;

use super::*;

fn event(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

fn plain(code: KeyCode) -> KeyEvent {
    event(code, KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    event(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn idle() -> KeyContext {
    KeyContext {
        composer_empty: true,
        at_line_start: true,
        at_word_start: true,
        history_browsable: true,
        running: false,
        plain: false,
    }
}

fn running() -> KeyContext {
    KeyContext {
        running: true,
        ..idle()
    }
}

fn composer(keymap: &mut Keymap, key: KeyEvent, ctx: KeyContext) -> Option<Action> {
    keymap
        .resolve(KeyArea::Composer, None, key, ctx, Instant::now())
        .map(|found| found.action)
}

fn load(name: &str) -> Keymap {
    Keymap::load(name).unwrap_or_else(|error| panic!("{name}: {error}"))
}

/// 사용자가 키로 하는 동작. 기본 표에서 빠지면 시험이 실패한다.
fn user_actions() -> Vec<Action> {
    vec![
        Action::ShowFullTranscript,
        Action::Suspend,
        Action::Submit,
        Action::SubmitQueued,
        Action::Newline,
        Action::HistoryPrev,
        Action::HistoryNext,
        Action::LineStart,
        Action::LineEnd,
        Action::KillToEnd,
        Action::KillToStart,
        Action::DeleteWordBack,
        Action::Yank,
        Action::HistorySearch,
        Action::ExternalEditor,
        Action::Redraw,
        Action::Quit,
        Action::Interrupt,
        Action::StopWork,
        Action::Rewind,
        Action::CyclePermissionMode,
        Action::CompleteCommand,
        Action::EnterBoard,
        Action::OpenTaskList,
        Action::Up,
        Action::Down,
        Action::Confirm,
        Action::Close,
    ]
}

#[test]
fn preset_names_match_the_protocol_list_and_all_load() {
    let registered: Vec<&str> = PRESETS.iter().map(|preset| preset.name).collect();

    assert_eq!(registered, PRESET_NAMES);
    for name in PRESET_NAMES {
        load(name);
    }
}

#[test]
fn every_binding_in_every_preset_has_a_key_or_a_command() {
    for name in PRESET_NAMES {
        let keymap = load(name);
        for binding in keymap.bindings() {
            assert!(
                !binding.keys.is_empty() || binding.command.is_some(),
                "{name}: {:?} has neither a key nor a command",
                binding.action
            );
        }
    }
}

#[test]
fn every_user_action_is_bound_in_every_preset() {
    for name in PRESET_NAMES {
        let keymap = load(name);
        for action in user_actions() {
            let bound = keymap.bindings().iter().any(|binding| {
                binding.action == action && (!binding.keys.is_empty() || binding.command.is_some())
            });
            assert!(bound, "{name}: {action:?} lost its key and command");
        }
    }
}

#[test]
fn commands_are_the_same_in_every_preset() {
    let commands = |keymap: &Keymap| {
        let mut commands: Vec<&str> = keymap
            .bindings()
            .iter()
            .filter_map(|binding| binding.command)
            .collect();
        commands.sort_unstable();
        commands.dedup();
        commands
    };
    let base = commands(&load("saturn"));

    for name in PRESET_NAMES {
        assert_eq!(commands(&load(name)), base, "{name}");
    }
}

#[test]
fn no_preset_binds_one_key_to_two_actions_in_a_scope() {
    // `Keymap::load`가 겹침을 오류로 돌려주므로 모든 프리셋이 읽히면 겹침이 없다. 겹침을 잡는지도 함께 본다.
    let clash = vec![
        Binding::new(Action::Submit, &[KeyArea::Composer], &["enter"]),
        Binding::new(Action::Newline, &[KeyArea::Composer], &["enter"]),
    ];

    let error = Keymap::from_tables("clash", clash, Vec::new()).unwrap_err();

    assert!(matches!(
        error,
        KeymapError::Conflict {
            scope: KeyArea::Composer,
            ..
        }
    ));
    for name in PRESET_NAMES {
        assert!(Keymap::load(name).is_ok(), "{name}");
    }
}

#[test]
fn a_condition_does_not_clash_with_an_unconditional_key() {
    let table = vec![
        Binding::new(Action::SubmitQueued, &[KeyArea::Composer], &["tab"]).when(Cond::Running),
        Binding::new(Action::CompleteCommand, &[KeyArea::Composer], &["tab"]),
    ];

    let mut keymap = Keymap::from_tables("when", table, Vec::new()).unwrap();

    assert_eq!(
        composer(&mut keymap, plain(KeyCode::Tab), running()),
        Some(Action::SubmitQueued)
    );
    assert_eq!(
        composer(&mut keymap, plain(KeyCode::Tab), idle()),
        Some(Action::CompleteCommand)
    );
}

#[test]
fn unreadable_keys_and_unknown_presets_are_errors() {
    let bad = vec![Binding::new(
        Action::Submit,
        &[KeyArea::Composer],
        &["ctrl+nope"],
    )];

    assert!(matches!(
        Keymap::from_tables("bad", bad, Vec::new()),
        Err(KeymapError::BadKey { .. })
    ));
    assert!(matches!(
        Keymap::load("vim"),
        Err(KeymapError::UnknownPreset { .. })
    ));
}

#[test]
fn an_override_replaces_only_its_own_keys_and_inherits_the_rest() {
    let base = vec![
        Binding::new(Action::Submit, &[KeyArea::Composer], &["enter"]),
        Binding::new(Action::Newline, &[KeyArea::Composer], &["ctrl+j"]),
        Binding::new(Action::Redraw, &[KeyArea::Composer], &["ctrl+l"]).command("/redraw"),
    ];
    let over = vec![Binding::new(
        Action::Redraw,
        &[KeyArea::Composer],
        &["ctrl+r"],
    )];

    let mut keymap = Keymap::from_tables("child", base, over).unwrap();

    assert_eq!(
        composer(&mut keymap, plain(KeyCode::Enter), idle()),
        Some(Action::Submit)
    );
    assert_eq!(
        composer(&mut keymap, ctrl('j'), idle()),
        Some(Action::Newline)
    );
    assert_eq!(
        composer(&mut keymap, ctrl('r'), idle()),
        Some(Action::Redraw)
    );
    assert_eq!(composer(&mut keymap, ctrl('l'), idle()), None);
    let redraw = keymap
        .bindings()
        .iter()
        .find(|binding| binding.action == Action::Redraw)
        .unwrap();
    assert_eq!(redraw.command, Some("/redraw"));
}

#[test]
fn common_keys_mean_the_same_in_every_preset() {
    let common = [
        (plain(KeyCode::Enter), Action::Submit),
        (ctrl('j'), Action::Newline),
        (plain(KeyCode::Up), Action::HistoryPrev),
        (plain(KeyCode::Down), Action::HistoryNext),
        (ctrl('a'), Action::LineStart),
        (ctrl('e'), Action::LineEnd),
        (ctrl('k'), Action::KillToEnd),
        (ctrl('u'), Action::KillToStart),
        (ctrl('w'), Action::DeleteWordBack),
        (ctrl('r'), Action::HistorySearch),
        (ctrl('g'), Action::ExternalEditor),
        (ctrl('l'), Action::Redraw),
        (ctrl('d'), Action::Quit),
        (
            plain(KeyCode::Char('/')),
            Action::OpenPopup(PopupKind::Command),
        ),
        (plain(KeyCode::F(3)), Action::EnterBoard),
        (plain(KeyCode::F(5)), Action::OpenTaskList),
        (plain(KeyCode::Left), Action::CursorLeft),
    ];
    for name in PRESET_NAMES {
        let mut keymap = load(name);
        for (key, action) in &common {
            let found = composer(&mut keymap, *key, idle());
            assert_eq!(found.as_ref(), Some(action), "{name} {key:?}");
        }
    }
}

#[test]
fn choice_keys_mean_the_same_in_every_preset() {
    for name in PRESET_NAMES {
        let mut keymap = load(name);
        let mut at = |area: KeyArea, key: KeyEvent| {
            keymap
                .resolve(area, Some(KeyArea::Composer), key, idle(), Instant::now())
                .map(|found| found.action)
        };

        for area in [
            KeyArea::Transcript,
            KeyArea::Correction,
            KeyArea::ModelPicker,
            KeyArea::StopConfirm,
            KeyArea::ConstraintAsk,
        ] {
            assert_eq!(
                at(area, plain(KeyCode::Up)),
                Some(Action::Up),
                "{name} {area:?}"
            );
            assert_eq!(
                at(area, plain(KeyCode::Down)),
                Some(Action::Down),
                "{name} {area:?}"
            );
            assert_eq!(
                at(area, plain(KeyCode::Enter)),
                Some(Action::Confirm),
                "{name} {area:?}"
            );
            assert_eq!(
                at(area, plain(KeyCode::Esc)),
                Some(Action::Close),
                "{name} {area:?}"
            );
        }
    }
}

#[test]
fn ctrl_d_quits_only_while_the_composer_is_empty() {
    let mut keymap = load("saturn");
    let filled = KeyContext {
        composer_empty: false,
        ..idle()
    };

    assert_eq!(composer(&mut keymap, ctrl('d'), filled), None);
    assert_eq!(composer(&mut keymap, ctrl('d'), idle()), Some(Action::Quit));
}

#[test]
fn saturn_keys_that_split_the_tools() {
    let mut keymap = load("saturn");

    assert_eq!(
        composer(&mut keymap, plain(KeyCode::Esc), idle()),
        Some(Action::StopWork)
    );
    assert_eq!(
        composer(
            &mut keymap,
            event(KeyCode::BackTab, KeyModifiers::SHIFT),
            idle()
        ),
        Some(Action::CyclePermissionMode)
    );
    assert_eq!(
        composer(&mut keymap, plain(KeyCode::Tab), idle()),
        Some(Action::CompleteCommand)
    );
    let found = keymap
        .resolve(KeyArea::Composer, None, ctrl('c'), idle(), Instant::now())
        .unwrap();
    assert_eq!(
        (found.action, found.scope),
        (Action::Interrupt, KeyArea::Fallback)
    );
}

#[test]
fn shift_tab_arrives_as_back_tab_or_shift_tab() {
    let mut keymap = load("saturn");

    let back_tab = composer(
        &mut keymap,
        event(KeyCode::BackTab, KeyModifiers::SHIFT),
        idle(),
    );
    let shifted = composer(
        &mut keymap,
        event(KeyCode::Tab, KeyModifiers::SHIFT),
        idle(),
    );

    assert_eq!(back_tab, Some(Action::CyclePermissionMode));
    assert_eq!(shifted, Some(Action::CyclePermissionMode));
}

#[test]
fn double_escape_means_rewind_only_inside_the_window() {
    let mut keymap = load("saturn");
    let now = Instant::now();
    let at = |keymap: &mut Keymap, after: u64| {
        keymap
            .resolve(
                KeyArea::Composer,
                None,
                plain(KeyCode::Esc),
                idle(),
                now + Duration::from_millis(after),
            )
            .map(|found| found.action)
    };

    let first = at(&mut keymap, 0);
    let second = at(&mut keymap, 300);
    let third = at(&mut keymap, 400);
    let slow = at(&mut keymap, 400 + 600);

    assert_eq!(first, Some(Action::StopWork));
    assert_eq!(second, Some(Action::Rewind));
    assert_eq!(third, Some(Action::StopWork));
    assert_eq!(slow, Some(Action::StopWork));
}

#[test]
fn another_key_between_two_escapes_breaks_the_pair() {
    let mut keymap = load("saturn");
    let now = Instant::now();
    let mut press = |key: KeyEvent, after: u64| {
        keymap
            .resolve(
                KeyArea::Composer,
                None,
                key,
                idle(),
                now + Duration::from_millis(after),
            )
            .map(|found| found.action)
    };

    press(plain(KeyCode::Esc), 0);
    press(plain(KeyCode::Char('x')), 100);

    assert_eq!(press(plain(KeyCode::Esc), 200), Some(Action::StopWork));
}

#[test]
fn switching_presets_changes_escape_tab_shift_tab_and_ctrl_c() {
    let mut saturn = load("saturn");
    let mut gemini = load("gemini");
    let mut opencode = load("opencode");
    let mut claude = load("claude");
    let shift_tab = event(KeyCode::BackTab, KeyModifiers::SHIFT);

    // Esc: Gemini CLI는 작업을 멈추지 않는다.
    assert_eq!(
        composer(&mut saturn, plain(KeyCode::Esc), running()),
        Some(Action::StopWork)
    );
    assert_eq!(
        composer(&mut gemini, plain(KeyCode::Esc), running()),
        Some(Action::ClearSelection)
    );
    // Tab: Gemini CLI는 작업 중이면 대기, OpenCode는 모드 순환.
    assert_eq!(
        composer(&mut saturn, plain(KeyCode::Tab), running()),
        Some(Action::CompleteCommand)
    );
    assert_eq!(
        composer(&mut gemini, plain(KeyCode::Tab), running()),
        Some(Action::SubmitQueued)
    );
    assert_eq!(
        composer(&mut gemini, plain(KeyCode::Tab), idle()),
        Some(Action::CompleteCommand)
    );
    assert_eq!(
        composer(&mut opencode, plain(KeyCode::Tab), idle()),
        Some(Action::CyclePermissionMode)
    );
    // Shift+Tab은 OpenCode에서도 모드 순환이다.
    assert_eq!(
        composer(&mut opencode, shift_tab, idle()),
        Some(Action::CyclePermissionMode)
    );
    // Ctrl+C: Gemini CLI는 빈 입력창에서 바로 종료.
    let ctrl_c = |keymap: &mut Keymap| {
        keymap
            .resolve(KeyArea::Composer, None, ctrl('c'), idle(), Instant::now())
            .map(|found| found.action)
    };
    assert_eq!(ctrl_c(&mut saturn), Some(Action::Interrupt));
    assert_eq!(ctrl_c(&mut gemini), Some(Action::InterruptQuit));
    // 전체 기록: Claude Code와 Gemini CLI는 Ctrl+O, 기본은 Ctrl+T.
    let transcript = |keymap: &mut Keymap, c: char| {
        keymap
            .resolve(KeyArea::Composer, None, ctrl(c), idle(), Instant::now())
            .map(|found| found.action)
    };
    assert_eq!(
        transcript(&mut saturn, 't'),
        Some(Action::ShowFullTranscript)
    );
    assert_eq!(
        transcript(&mut claude, 'o'),
        Some(Action::ShowFullTranscript)
    );
    assert_eq!(transcript(&mut claude, 't'), None);
    assert_eq!(
        transcript(&mut gemini, 'o'),
        Some(Action::ShowFullTranscript)
    );
}

#[test]
fn opencode_has_no_double_escape_and_gemini_moves_yank() {
    let mut opencode = load("opencode");
    let mut gemini = load("gemini");
    let now = Instant::now();

    opencode.resolve(KeyArea::Composer, None, plain(KeyCode::Esc), idle(), now);
    let second = opencode.resolve(KeyArea::Composer, None, plain(KeyCode::Esc), idle(), now);

    assert_eq!(second.map(|found| found.action), Some(Action::StopWork));
    assert_eq!(composer(&mut gemini, ctrl('y'), idle()), None);
    assert_eq!(
        composer(
            &mut gemini,
            event(KeyCode::Char('y'), KeyModifiers::ALT),
            idle()
        ),
        Some(Action::Yank)
    );
}

#[test]
fn typing_falls_back_to_text_in_text_areas_only() {
    let mut keymap = load("saturn");
    let now = Instant::now();

    let search = keymap.resolve(
        KeyArea::Search,
        Some(KeyArea::Composer),
        plain(KeyCode::Char('x')),
        idle(),
        now,
    );
    let choice = keymap.resolve(
        KeyArea::Correction,
        Some(KeyArea::Composer),
        plain(KeyCode::Char('x')),
        idle(),
        now,
    );
    let list = keymap.resolve(
        KeyArea::ModelPicker,
        None,
        plain(KeyCode::Char('x')),
        idle(),
        now,
    );

    assert_eq!(search.map(|found| found.action), Some(Action::Insert('x')));
    let choice = choice.unwrap();
    assert_eq!(
        (choice.action, choice.scope),
        (Action::Insert('x'), KeyArea::Composer)
    );
    assert_eq!(list, None);
}

#[test]
fn global_keys_win_in_every_area() {
    let mut keymap = load("saturn");

    for area in [
        KeyArea::Permission,
        KeyArea::TaskList,
        KeyArea::Input,
        KeyArea::RouterKeyPrompt,
    ] {
        let found = keymap.resolve(area, None, ctrl('t'), idle(), Instant::now());
        assert_eq!(
            found.map(|found| found.action),
            Some(Action::ShowFullTranscript),
            "{area:?}"
        );
    }
}

fn sources(dir: &std::path::Path, found: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, found);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            found.push(path);
        }
    }
}

/// 키를 동작으로 바꾸는 일은 `keymap` 한 곳이 맡는다. 다른 파일의 제품 코드는 `KeyCode`와 `KeyModifiers`를 읽지 않는다.
#[test]
fn only_the_keymap_module_reads_key_codes() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    sources(&root, &mut files);
    let mut offenders = Vec::new();

    for path in files {
        let relative = path
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        if relative.starts_with("keymap")
            || relative.ends_with("tests.rs")
            || relative == "terminal.rs"
        {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let product = text.split("#[cfg(test)]").next().unwrap_or_default();
        if product.contains("KeyCode::") || product.contains("KeyModifiers::") {
            offenders.push(relative);
        }
    }

    assert!(
        offenders.is_empty(),
        "raw key reads outside keymap: {offenders:?}"
    );
}

const TUI_DOC: &str = include_str!("../../../../docs/design/tui.md");

#[test]
fn the_design_doc_names_every_preset_and_slash_command() {
    for name in PRESET_NAMES {
        assert!(
            TUI_DOC.contains(&format!("`{name}`")),
            "preset {name} is not in tui.md"
        );
    }
    for spec in crate::commands::SATURN_COMMANDS {
        let command = format!("`/{}", spec.path);
        assert!(
            TUI_DOC.contains(&command),
            "command /{} is not in tui.md",
            spec.path
        );
    }
}

#[test]
fn every_command_in_the_key_table_is_a_real_command() {
    for binding in load("saturn").bindings() {
        if let Some(command) = binding.command {
            let path = command.trim_start_matches('/');
            assert!(
                crate::commands::SATURN_COMMANDS
                    .iter()
                    .any(|spec| spec.path == path),
                "{command} is bound but is not a command"
            );
        }
    }
}
