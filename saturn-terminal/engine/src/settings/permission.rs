//! 설정의 `permission` 키: 층마다 읽어 모드와 규칙 목록으로 합친다.
//! 설계: docs/design/settings.md#설정-키, docs/design/permissions.md#권한-규칙

use saturn_core::permission::{Mode, PermissionTool, Rule, Verdict, parse_tool};
use serde_json::{Value, json};

use super::{Layer, LayerSource};

/// 병합 결과에서 `permission` 표가 놓이는 키.
pub(super) const KEY: &str = "permission";

/// 모드 키 이름.
pub(super) const MODE_KEY: &str = "permission.mode";

/// 도구 전체에 적용하는 규칙(`permission.shell = "ask"`)의 패턴.
const WHOLE_TOOL_PATTERN: &str = "*";

/// 한 층이 적은 모드와 규칙. 규칙은 파일에 적힌 순서다.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct LayerPermission {
    pub(super) mode: Option<Mode>,
    pub(super) rules: Vec<Rule>,
}

/// 병합이 끝난 모드와 규칙. 규칙은 사용자, 폴더, 채팅, 실행 층 순서로 이어 붙인 목록이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PermissionSettings {
    pub mode: Mode,
    pub rules: Vec<Rule>,
}

impl Default for PermissionSettings {
    fn default() -> Self {
        Self {
            mode: Mode::DEFAULT,
            rules: Vec::new(),
        }
    }
}

// cost: time O(k), heap O(k), stack O(1), alloc k
// vars: k = permission 키 수
// basis: estimate
/// 층의 TOML 원문에서 `permission` 표를 읽는다. 문법은 이미 검사했다고 본다.
///
/// # Errors
/// 모르는 키나 값이면 `(키, 이유)`.
pub(super) fn read_layer(content: &str) -> Result<LayerPermission, (String, String)> {
    let Ok(doc) = content.parse::<toml_edit::DocumentMut>() else {
        return Ok(LayerPermission::default());
    };
    let Some(item) = doc.get(KEY) else {
        return Ok(LayerPermission::default());
    };
    let table = item
        .as_table_like()
        .ok_or_else(|| (KEY.to_owned(), "expected a table".to_owned()))?;
    let mut layer = LayerPermission::default();
    for (key, item) in table.iter() {
        match key {
            "mode" => layer.mode = Some(read_mode(item)?),
            _ => {
                let tool = parse_tool(key)
                    .ok_or_else(|| (format!("{KEY}.{key}"), "unknown key".to_owned()))?;
                read_rules(key, tool, item, &mut layer.rules)?;
            }
        }
    }
    Ok(layer)
}

fn read_mode(item: &toml_edit::Item) -> Result<Mode, (String, String)> {
    let invalid = |reason: &str| (MODE_KEY.to_owned(), reason.to_owned());
    let text = item.as_str().ok_or_else(|| invalid("expected a string"))?;
    Mode::parse(text).ok_or_else(|| invalid("expected one of ask, edit, read-only, full"))
}

// cost: time O(k), heap O(k), stack O(1), alloc k
// vars: k = 도구 아래 패턴 수
// basis: estimate
fn read_rules(
    key: &str,
    tool: PermissionTool,
    item: &toml_edit::Item,
    rules: &mut Vec<Rule>,
) -> Result<(), (String, String)> {
    let path = format!("{KEY}.{key}");
    if let Some(text) = item.as_str() {
        rules.push(rule(tool, WHOLE_TOOL_PATTERN, read_verdict(&path, text)?));
        return Ok(());
    }
    let table = item
        .as_table_like()
        .ok_or_else(|| (path.clone(), "expected a string or a table".to_owned()))?;
    for (pattern, value) in table.iter() {
        let pattern_path = format!("{path}.{pattern}");
        let text = value
            .as_str()
            .ok_or_else(|| (pattern_path.clone(), "expected a string".to_owned()))?;
        rules.push(rule(tool, pattern, read_verdict(&pattern_path, text)?));
    }
    Ok(())
}

fn read_verdict(path: &str, text: &str) -> Result<Verdict, (String, String)> {
    Verdict::parse(text).ok_or_else(|| {
        (
            path.to_owned(),
            "expected one of allow, ask, deny".to_owned(),
        )
    })
}

fn rule(tool: PermissionTool, pattern: &str, verdict: Verdict) -> Rule {
    Rule {
        tool,
        pattern: pattern.to_owned(),
        verdict,
    }
}

// cost: time O(l + r), heap O(r), stack O(1), alloc r
// vars: l = 층 수, r = 규칙 수
// basis: estimate
/// 층 순서대로 모드와 규칙을 합친다. 폴더 층의 모드가 앞 층까지 합친 모드보다 같거나 높으면 무시하고
/// 그 층의 `ignored`에 `permission.mode`를 남긴다.
pub(super) fn fold(
    sources: &mut [LayerSource],
    permissions: Vec<LayerPermission>,
) -> PermissionSettings {
    let mut folded = PermissionSettings::default();
    for (source, mut permission) in sources.iter_mut().zip(permissions) {
        if let Some(mode) = permission.mode {
            if source.layer != Layer::Folder {
                folded.mode = mode;
            } else if let Some(lowered) = folded.mode.lowered_by(mode) {
                folded.mode = lowered;
            } else {
                source.ignored.push(MODE_KEY.to_owned());
            }
        }
        folded.rules.append(&mut permission.rules);
    }
    folded
}

// cost: time O(r), heap O(r), stack O(1), alloc r
// vars: r = 규칙 수
// basis: estimate
/// 병합 결과에 넣는 값. 모드 이름과 규칙 목록 하나다.
pub(super) fn to_value(settings: &PermissionSettings) -> Value {
    let rules: Vec<Value> = settings
        .rules
        .iter()
        .map(|rule| {
            json!({
                "tool": saturn_core::permission::tool_name(rule.tool),
                "pattern": rule.pattern,
                "verdict": rule.verdict.name(),
            })
        })
        .collect();
    json!({ "mode": settings.mode.name(), "rules": rules })
}

// cost: time O(r), heap O(r), stack O(1), alloc r
// vars: r = 규칙 수
// basis: estimate
/// 병합 결과의 `permission` 표를 읽는다. 없거나 깨졌으면 기본값이다.
pub(super) fn from_value(value: Option<&Value>) -> PermissionSettings {
    let Some(value) = value else {
        return PermissionSettings::default();
    };
    let mode = value["mode"]
        .as_str()
        .and_then(Mode::parse)
        .unwrap_or(Mode::DEFAULT);
    let rules = value["rules"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            Some(Rule {
                tool: parse_tool(item["tool"].as_str()?)?,
                pattern: item["pattern"].as_str()?.to_owned(),
                verdict: Verdict::parse(item["verdict"].as_str()?)?,
            })
        })
        .collect();
    PermissionSettings { mode, rules }
}

/// 채팅 층 원문이 적은 모드. 없거나 읽지 못하면 `None`.
pub(crate) fn chat_layer_mode(content: Option<&str>) -> Option<Mode> {
    read_layer(content?).ok()?.mode
}

/// 채팅 층 원문의 `permission.mode`만 바꾸고 나머지는 그대로 둔다.
pub(crate) fn with_chat_layer_mode(content: Option<&str>, mode: Mode) -> String {
    let mut doc = content
        .and_then(|content| content.parse::<toml_edit::DocumentMut>().ok())
        .unwrap_or_default();
    if let Ok(keys) = super::layers::parse_key(MODE_KEY) {
        let value = toml_edit::Value::from(mode.name());
        super::layers::set_path(doc.as_table_mut(), &keys, value);
    }
    doc.to_string()
}

/// 폴더 설정의 모드가 사용자 층까지 합친 모드보다 같거나 높아 무시되는지. 신뢰 창이 쓴다.
pub(super) fn is_folder_mode_ignored(user_content: Option<&str>, folder_content: &str) -> bool {
    let Some(folder) = read_layer(folder_content).ok().and_then(|layer| layer.mode) else {
        return false;
    };
    let user = user_content
        .and_then(|content| read_layer(content).ok())
        .and_then(|layer| layer.mode)
        .unwrap_or(Mode::DEFAULT);
    user.lowered_by(folder).is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(content: &str) -> LayerPermission {
        read_layer(content).unwrap()
    }

    #[test]
    fn read_layer_keeps_file_order_and_whole_tool_rules() {
        let layer = parsed(
            "[permission]\nmode = \"ask\"\nmcp = \"deny\"\n\
             [permission.shell]\n\"git status\" = \"allow\"\n\"rm *\" = \"deny\"\n\"cargo *\" = \"ask\"\n",
        );

        assert_eq!(layer.mode, Some(Mode::Ask));
        let rules: Vec<(PermissionTool, &str, Verdict)> = layer
            .rules
            .iter()
            .map(|rule| (rule.tool, rule.pattern.as_str(), rule.verdict))
            .collect();
        assert_eq!(
            rules,
            vec![
                (PermissionTool::Mcp, "*", Verdict::Deny),
                (PermissionTool::Shell, "git status", Verdict::Allow),
                (PermissionTool::Shell, "rm *", Verdict::Deny),
                (PermissionTool::Shell, "cargo *", Verdict::Ask),
            ]
        );
    }

    #[test]
    fn read_layer_accepts_read_rules() {
        let layer =
            parsed("[permission.read]\n\"/etc/*\" = \"allow\"\n\"/etc/shadow\" = \"deny\"\n");

        let rules: Vec<(PermissionTool, &str, Verdict)> = layer
            .rules
            .iter()
            .map(|rule| (rule.tool, rule.pattern.as_str(), rule.verdict))
            .collect();
        assert_eq!(
            rules,
            vec![
                (PermissionTool::Read, "/etc/*", Verdict::Allow),
                (PermissionTool::Read, "/etc/shadow", Verdict::Deny),
            ]
        );
    }

    #[test]
    fn read_layer_accepts_inline_table_and_dotted_keys() {
        let layer =
            parsed("permission.shell = { \"ls\" = \"allow\" }\npermission.edit = \"ask\"\n");

        assert_eq!(layer.rules.len(), 2);
    }

    #[test]
    fn read_layer_rejects_bad_keys_and_values() {
        let cases = [
            ("permission.mode = \"plan\"\n", "permission.mode"),
            ("permission.mode = 1\n", "permission.mode"),
            ("permission.net = \"allow\"\n", "permission.net"),
            ("permission.shell = \"maybe\"\n", "permission.shell"),
            ("permission.shell = 3\n", "permission.shell"),
            ("[permission.shell]\n\"ls\" = 1\n", "permission.shell.ls"),
            ("permission = \"allow\"\n", "permission"),
        ];
        for (content, key) in cases {
            let error = read_layer(content).unwrap_err();

            assert_eq!(error.0, key, "{content}");
        }
    }

    #[test]
    fn fold_applies_folder_mode_only_when_lower() {
        let source = |layer| LayerSource {
            layer,
            path: None,
            fingerprint: None,
            ignored: Vec::new(),
        };
        let with_mode = |mode| LayerPermission {
            mode: Some(mode),
            rules: Vec::new(),
        };
        let mut raised = vec![source(Layer::User), source(Layer::Folder)];
        let mut lowered = vec![
            source(Layer::User),
            source(Layer::Folder),
            source(Layer::Chat),
        ];

        let kept = fold(
            &mut raised,
            vec![with_mode(Mode::Edit), with_mode(Mode::Full)],
        );
        let dropped = fold(
            &mut lowered,
            vec![
                with_mode(Mode::Edit),
                with_mode(Mode::ReadOnly),
                with_mode(Mode::Full),
            ],
        );

        assert_eq!(kept.mode, Mode::Edit);
        assert_eq!(raised[1].ignored, vec![MODE_KEY.to_owned()]);
        assert_eq!(dropped.mode, Mode::Full);
        assert!(lowered[1].ignored.is_empty());
    }

    #[test]
    fn value_round_trips() {
        let settings = PermissionSettings {
            mode: Mode::Ask,
            rules: parsed("permission.mcp = \"deny\"\n[permission.shell]\n\"ls *\" = \"allow\"\n")
                .rules,
        };

        assert_eq!(from_value(Some(&to_value(&settings))), settings);
        assert_eq!(from_value(None), PermissionSettings::default());
    }

    #[test]
    fn folder_mode_is_ignored_unless_lower_than_the_user_mode() {
        assert!(is_folder_mode_ignored(None, "permission.mode = \"full\"\n"));
        assert!(is_folder_mode_ignored(
            Some("permission.mode = \"ask\"\n"),
            "permission.mode = \"ask\"\n"
        ));
        assert!(!is_folder_mode_ignored(None, "permission.mode = \"ask\"\n"));
        assert!(!is_folder_mode_ignored(None, "on_exit = \"ask\"\n"));
    }
}
