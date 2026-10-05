//! 설정 이름 규칙 시험과 옛 이름 별칭 시험. 규칙은 docs/design/settings.md의 이름 규칙 절이 정본이다.
//! 설계: docs/design/settings.md#이름-규칙

use std::path::PathBuf;

use serde_json::Value;

use super::layers::{
    Kind, PROVIDER_SCHEMA, RENAMED_KEYS, SCHEMA, USER_ONLY, default_layer, get_path, is_user_only,
    merge, source,
};
use super::{Layer, LayerSource, Screen, SettingsError};
use crate::providers::ContextDefaults;

const SETTINGS_DOC: &str = include_str!("../../../../docs/design/settings.md");

const DEFAULTS: ContextDefaults = ContextDefaults {
    window: 200_000,
    cache_write: 1.25,
};

fn layer(layer: Layer, content: &str) -> (LayerSource, String) {
    let path = matches!(layer, Layer::User | Layer::Folder)
        .then(|| PathBuf::from(format!("/{layer:?}/.saturn/config.toml")));
    (source(layer, path, content), content.to_owned())
}

fn merged(layers: &[(Layer, &str)]) -> Result<super::SettingsSnapshot, SettingsError> {
    let mut all = vec![layer(Layer::Default, default_layer())];
    all.extend(layers.iter().map(|(which, content)| layer(*which, content)));
    merge(all)
}

fn accepts(content: &str) -> bool {
    merged(&[(Layer::User, content)]).is_ok()
}

/// 모든 스키마 키. provider 키는 `provider.<id>.` 뒤 경로에 `provider.codex.`를 붙여 센다.
fn all_keys() -> Vec<(String, Kind)> {
    let mut keys: Vec<(String, Kind)> = SCHEMA
        .iter()
        .map(|(name, kind)| ((*name).to_owned(), *kind))
        .collect();
    keys.extend(
        PROVIDER_SCHEMA
            .iter()
            .map(|(name, kind)| (format!("provider.codex.{name}"), *kind)),
    );
    keys
}

fn segments(key: &str) -> Vec<&str> {
    key.split('.').collect()
}

// 규칙 0: 소문자 영문, 숫자, 밑줄, 점만 쓴다.
#[test]
fn every_key_uses_lowercase_digits_underscores_and_dots() {
    for (key, _) in all_keys() {
        assert!(
            key.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.'),
            "{key}"
        );
        assert!(segments(&key).iter().all(|part| !part.is_empty()), "{key}");
    }
}

// 규칙 1: 묶음 없는 키는 두지 않는다.
#[test]
fn every_key_has_a_group() {
    for (key, _) in all_keys() {
        assert!(segments(&key).len() >= 2, "{key} has no group");
    }
}

// 규칙 2: 맨 앞 묶음은 단수다.
#[test]
fn group_names_are_singular() {
    for (key, _) in all_keys() {
        let group = segments(&key)[0];
        assert!(!group.ends_with('s'), "{key}: group {group} is plural");
    }
}

// 규칙 3: 켜고 끄는 값은 접두사 없이 켜짐 뜻 이름이고 기본은 끔이다.
#[test]
fn flags_have_no_prefix_and_default_to_off() {
    let defaults = merged(&[]).unwrap();
    for (key, kind) in all_keys() {
        if !matches!(kind, Kind::Flag) {
            continue;
        }
        let name = *segments(&key).last().unwrap();
        for prefix in ["enable_", "is_", "use_", "has_", "disable_", "no_"] {
            assert!(!name.starts_with(prefix), "{key}: flag prefix {prefix}");
        }
        if let Some(value) = get_path(&defaults.settings.values, &key) {
            assert_eq!(value, &Value::Bool(false), "{key} should default to off");
        }
    }
}

// 규칙 4: 단위는 정해진 접미사로만 쓴다.
#[test]
fn unit_suffixes_are_the_fixed_five() {
    const UNITS: [&str; 5] = ["_days", "_sec", "_ms", "_bytes", "_percent"];
    const FORBIDDEN: [&str; 14] = [
        "_day", "_secs", "_seconds", "_second", "_millis", "_minutes", "_hours", "_byte", "_pct",
        "_divisor", "_ratio", "_kb", "_mb", "_timeout",
    ];
    for (key, kind) in all_keys() {
        let name = *segments(&key).last().unwrap();
        for suffix in FORBIDDEN {
            assert!(!name.ends_with(suffix), "{key}: use one of {UNITS:?}");
        }
        let percent_kind = matches!(kind, Kind::Percent | Kind::PercentFrom1);
        assert_eq!(
            name.ends_with("_percent"),
            percent_kind,
            "{key}: `_percent` keys are integer percents and the reverse"
        );
        if name.ends_with("_days") {
            assert!(matches!(kind, Kind::Positive | Kind::Whole), "{key}");
        }
    }
}

// 규칙 5: 고르는 값은 소문자 영문이고 두 단어는 `-`로 잇는다.
#[test]
fn choices_are_lowercase_with_hyphens() {
    for (key, kind) in all_keys() {
        let Kind::OneOf(options) = kind else {
            continue;
        };
        for option in options {
            assert!(
                !option.is_empty()
                    && option
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "{key}: choice {option}"
            );
            assert!(
                !option.starts_with('-') && !option.ends_with('-') && !option.contains("--"),
                "{key}: choice {option}"
            );
        }
    }
}

// 규칙 6: 동작 방식은 `mode`다.
#[test]
fn behavior_choices_are_named_mode() {
    for (key, _) in all_keys() {
        let name = *segments(&key).last().unwrap();
        for word in ["method", "type", "strategy", "style", "kind"] {
            assert_ne!(name, word, "{key}: name behavior choices `mode`");
        }
    }
}

// 규칙 7: 보안과 비용 키는 사용자 설정에서만 정한다.
#[test]
fn security_and_cost_keys_are_user_only() {
    for key in [
        "router.endpoint",
        "router.key.command",
        "router.key.storage",
        "router.key.info.source",
        "router.mode",
        "grading.model",
        "consent.share_with_server",
        "retention.max_age_days",
        "retention.auto_prune",
        "debug.provider_events",
        "constraint.auto_apply",
    ] {
        assert!(is_user_only(key), "{key}");
    }
    assert!(USER_ONLY.len() >= 6);
    for key in ["tui.on_exit", "context.mode", "model.mode", "tui.screen"] {
        assert!(!is_user_only(key), "{key}");
    }
}

// 규칙 8: 위치는 설정 키가 아니다.
#[test]
fn no_key_names_a_location() {
    for (key, _) in all_keys() {
        let name = *segments(&key).last().unwrap();
        for suffix in ["_dir", "_path", "_file", "_home", "_folder"] {
            assert!(!name.ends_with(suffix), "{key}: locations use SATURN_HOME");
        }
    }
}

#[test]
fn renamed_keys_follow_the_rules_and_are_not_in_the_schema() {
    for item in RENAMED_KEYS {
        assert!(
            SCHEMA.iter().any(|(name, _)| *name == item.new),
            "{} is not in the schema",
            item.new
        );
        assert!(
            SCHEMA.iter().all(|(name, _)| *name != item.old),
            "{} is old but still in the schema",
            item.old
        );
    }
}

fn documented_schema_keys() -> Vec<String> {
    let section = SETTINGS_DOC
        .split("### 설정 키\n")
        .nth(1)
        .and_then(|rest| rest.split("### 설정 키가 없는 고정 상수").next())
        .expect("settings doc should have the key section");
    section
        .lines()
        .filter(|line| line.starts_with("| `"))
        .filter(|line| !line.contains("구현 전("))
        .flat_map(|line| {
            let first = line.split(" | ").next().unwrap_or_default();
            first
                .split('`')
                .skip(1)
                .step_by(2)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .filter(|key| !key.contains('<') && !key.starts_with("permission."))
        .collect()
}

#[test]
fn doc_table_and_schema_list_the_same_keys() {
    let documented = documented_schema_keys();
    let covers = |doc: &str, key: &str| {
        doc == key
            || key
                .strip_prefix(doc)
                .is_some_and(|rest| rest.starts_with('.'))
    };
    for (key, _) in SCHEMA {
        let thresholds = key.starts_with("router.thresholds.");
        assert!(
            thresholds || documented.iter().any(|doc| covers(doc, key)),
            "{key} is in the schema but not in the doc table"
        );
    }
    for doc in &documented {
        assert!(
            SCHEMA.iter().any(|(key, _)| covers(doc, key)),
            "{doc} is in the doc table but not in the schema"
        );
    }
}

#[test]
fn doc_lists_the_eight_rules_and_every_alias() {
    let section = SETTINGS_DOC
        .split("### 이름 규칙\n")
        .nth(1)
        .and_then(|rest| rest.split("### 설정 키\n").next())
        .expect("settings doc should have the name rules");
    for number in 1..=8 {
        assert!(
            section.contains(&format!("| {number} | ")),
            "rule {number} is missing"
        );
    }
    assert!(section.contains("SATURN_HOME"));
    for item in RENAMED_KEYS {
        assert!(
            section.contains(&format!("`{}`", item.old))
                && section.contains(&format!("`{}`", item.new)),
            "{} -> {} is missing from the alias table",
            item.old,
            item.new
        );
    }
    for pending in [
        "context.constraint_slot_divisor",
        "context.constraint_slot_percent",
        "agents.worktree",
        "agent.worktree",
    ] {
        assert!(section.contains(pending), "{pending}");
    }
}

#[test]
fn doc_lists_the_fixed_constants() {
    let section = SETTINGS_DOC
        .split("### 설정 키가 없는 고정 상수")
        .nth(1)
        .and_then(|rest| rest.split("### provider 설정 키").next())
        .expect("settings doc should have the constants section");
    for name in [
        "RETRY_INTERVAL",
        "MAX_RETRIES",
        "RETRY_DEADLINE",
        "RetryPolicy",
    ] {
        assert!(section.contains(name), "{name}");
    }
    assert_eq!(saturn_core::routers::failure::RETRY_INTERVAL.as_secs(), 5);
    assert_eq!(saturn_core::routers::failure::MAX_RETRIES, 2);
    assert_eq!(saturn_core::routers::failure::RETRY_DEADLINE.as_secs(), 10);
    assert!(section.contains("v3"));
}

// 별칭 읽기와 경고

#[test]
fn old_names_are_read_as_the_new_names_with_a_warning_line() {
    let snapshot = merged(&[(
        Layer::User,
        "on_exit = \"stop\"\n[router]\nmethod = \"saturn\"\n[context]\npacket_hard_divisor = 4\n",
    )])
    .unwrap();

    assert_eq!(
        snapshot.settings.on_exit(),
        saturn_protocol::state::OnExit::Stop
    );
    assert_eq!(
        snapshot.settings.method(),
        saturn_core::routers::Method::Saturn
    );
    let budget = snapshot
        .settings
        .context_budget(crate::providers::test_support::CODEX, DEFAULTS);
    assert_eq!(budget.packet_hard_percent, 25);
    assert_eq!(get_path(&snapshot.settings.values, "on_exit"), None);
    assert_eq!(get_path(&snapshot.settings.values, "router.method"), None);
    let user = &snapshot.layers[1];
    assert_eq!(
        user.renamed,
        vec![
            ("on_exit".to_owned(), "tui.on_exit".to_owned()),
            ("router.method".to_owned(), "router.mode".to_owned()),
            (
                "context.packet_hard_divisor".to_owned(),
                "context.packet_hard_percent".to_owned()
            ),
        ]
    );
}

#[test]
fn new_name_wins_over_the_old_name_in_the_same_layer() {
    let snapshot = merged(&[(Layer::User, "on_exit = \"stop\"\ntui.on_exit = \"ask\"\n")]).unwrap();

    assert_eq!(
        snapshot.settings.on_exit(),
        saturn_protocol::state::OnExit::Ask
    );
}

#[test]
fn higher_layer_old_name_beats_lower_layer_new_name() {
    let snapshot = merged(&[
        (Layer::User, "tui.on_exit = \"ask\"\n"),
        (Layer::Chat, "on_exit = \"stop\"\n"),
    ])
    .unwrap();

    assert_eq!(
        snapshot.settings.on_exit(),
        saturn_protocol::state::OnExit::Stop
    );
}

#[test]
fn run_layer_old_name_is_read_too() {
    let content = super::layers::run_layer(&["on_exit=\"ask\"".to_owned()]).unwrap();
    let snapshot = merge(vec![
        layer(Layer::Default, default_layer()),
        (source(Layer::Run, None, &content), content),
    ])
    .unwrap();

    assert_eq!(
        snapshot.settings.on_exit(),
        saturn_protocol::state::OnExit::Ask
    );
}

#[test]
fn divisor_converts_to_percent_and_bad_values_fail_on_the_new_key() {
    let percent = |content: &str| {
        merged(&[(Layer::User, content)])
            .map(|snapshot| {
                snapshot
                    .settings
                    .context_budget(crate::providers::test_support::CODEX, DEFAULTS)
                    .packet_hard_percent
            })
            .map_err(|error| match error {
                SettingsError::Invalid { key, .. } => key,
                other => panic!("{other:?}"),
            })
    };

    assert_eq!(percent("context.packet_hard_divisor = 5\n"), Ok(20));
    assert_eq!(percent("context.packet_hard_divisor = 3\n"), Ok(33));
    assert_eq!(
        percent("context.packet_hard_divisor = 0\n"),
        Err("context.packet_hard_percent".to_owned())
    );
    assert_eq!(
        percent("context.packet_hard_divisor = 500\n"),
        Err("context.packet_hard_percent".to_owned())
    );
}

#[test]
fn old_snapshot_keys_are_still_read() {
    let mut snapshot = merged(&[]).unwrap();
    let Value::Object(values) = &mut snapshot.settings.values else {
        panic!("settings should be a table");
    };
    values.remove("notify");
    values["tui"].as_object_mut().unwrap().remove("on_exit");
    values["router"].as_object_mut().unwrap().remove("mode");
    values["context"]
        .as_object_mut()
        .unwrap()
        .remove("packet_hard_percent");
    values.insert("on_exit".to_owned(), Value::from("ask"));
    values["router"]
        .as_object_mut()
        .unwrap()
        .insert("method".to_owned(), Value::from("collect"));
    values["context"]
        .as_object_mut()
        .unwrap()
        .insert("packet_hard_divisor".to_owned(), Value::from(4));

    assert_eq!(
        snapshot.settings.on_exit(),
        saturn_protocol::state::OnExit::Ask
    );
    assert_eq!(
        snapshot.settings.method(),
        saturn_core::routers::Method::Collect
    );
    assert!(!snapshot.settings.notify_on_done());
    assert_eq!(
        snapshot
            .settings
            .context_budget(crate::providers::test_support::CODEX, DEFAULTS)
            .packet_hard_percent,
        25
    );
}

#[test]
fn key_source_reads_both_cases_and_stores_lowercase() {
    for (written, expected) in [("Stored", "stored"), ("env", "env"), ("Command", "command")] {
        let content = format!("[router.key.info]\nsource = \"{written}\"\nlast4 = \"abcd\"\n");
        let snapshot = merged(&[(Layer::User, &content)]).unwrap();

        assert_eq!(
            get_path(&snapshot.settings.values, "router.key.info.source"),
            Some(&Value::from(expected))
        );
        assert!(snapshot.settings.key_info().is_some(), "{written}");
    }
    let old: crate::secrets::KeyInfo =
        serde_json::from_str(r#"{"source":"Stored","last4":"abcd"}"#).unwrap();
    assert_eq!(old.source, crate::secrets::KeySource::Stored);
    assert_eq!(
        serde_json::to_string(&old).unwrap(),
        r#"{"source":"stored","last4":"abcd"}"#
    );
    assert!(!accepts("[router.key.info]\nsource = \"keychain\"\n"));
}

// 사용자 전용 목록

#[test]
fn folder_layer_cannot_set_retention_or_the_router_mode_under_either_name() {
    let snapshot = merged(&[
        (
            Layer::User,
            "retention.max_age_days = 30\n[router]\nmode = \"saturn\"\n",
        ),
        (
            Layer::Folder,
            "retention.max_age_days = 1\nretention.auto_prune = true\nrouter.method = \"collect\"\nrouter.mode = \"jev\"\n",
        ),
    ])
    .unwrap();

    assert_eq!(
        snapshot.settings.retention().max_age,
        Some(std::time::Duration::from_secs(30 * 24 * 60 * 60))
    );
    assert!(!snapshot.settings.retention().auto_prune);
    assert_eq!(
        snapshot.settings.method(),
        saturn_core::routers::Method::Saturn
    );
    let ignored = &snapshot.layers[2].ignored;
    for key in [
        "retention.max_age_days",
        "retention.auto_prune",
        "router.mode",
    ] {
        assert!(
            ignored.iter().any(|item| item == key),
            "{key} in {ignored:?}"
        );
    }
}

// tui.screen

#[test]
fn screen_defaults_to_auto_and_accepts_the_three_values() {
    assert_eq!(merged(&[]).unwrap().settings.screen(), Screen::Auto);
    for (value, expected) in [
        ("auto", Screen::Auto),
        ("full", Screen::Full),
        ("plain", Screen::Plain),
    ] {
        let snapshot = merged(&[(Layer::User, &format!("tui.screen = \"{value}\"\n"))]).unwrap();
        assert_eq!(snapshot.settings.screen(), expected);
    }
    assert!(!accepts("tui.screen = \"tiny\"\n"));
    assert!(!accepts("tui.screen = true\n"));
    assert!(accepts("[tui]\nscreen = \"plain\"\n"));
}

// notify.on_done

#[test]
fn notify_on_done_defaults_to_off_and_takes_a_boolean() {
    assert!(!merged(&[]).unwrap().settings.notify_on_done());
    let snapshot = merged(&[(Layer::User, "notify.on_done = true\n")]).unwrap();
    assert!(snapshot.settings.notify_on_done());
    assert!(!accepts("notify.on_done = \"yes\"\n"));
}

// 문서에 없는 옛 형식은 새 규칙을 어기므로 스키마에 다시 들어오면 안 된다.
#[test]
fn old_names_are_rejected_without_the_alias_path() {
    for item in RENAMED_KEYS {
        assert!(
            SCHEMA.iter().all(|(name, _)| *name != item.old),
            "{}",
            item.old
        );
    }
    assert!(!accepts("agents.worktree = true\n"));
    assert!(!accepts("agent.worktree = true\n"));
}
