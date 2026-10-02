//! Codex 권한 연결: 승인 요청을 규칙이 읽는 호출로 옮기고, 시작 때 적용 정책과 MCP 준비를 확인한다.
//! 설계: docs/design/permissions.md#codex-구성

use std::collections::HashMap;

use saturn_protocol::event::{PermissionCall, PermissionTool};
use serde_json::Value;

use super::tool_detail::unwrap_shell;

/// 모든 요청을 Saturn에 올리려고 `thread/start`에 주는 승인 정책. 설정 키로는 쓸 수 없다.
pub(super) const APPROVAL_POLICY: &str = "untrusted";

/// 파일 편집도 승인 요청으로 받으려고 주는 샌드박스.
pub(super) const SANDBOX: &str = "read-only";

const ELICITATION_METHOD: &str = "mcpServer/elicitation/request";

/// 도구 이름을 알 수 없는 MCP 요청의 도구 자리.
const UNKNOWN_MCP_TOOL: &str = "?";

/// `thread/start`와 `thread/resume`의 응답이 실제로 적용한 승인 정책, 샌드박스, 검토자가 기대와 다르면 이유 한 줄.
pub(super) fn check_applied(result: &Value) -> Result<(), String> {
    let policy = super::codex::value_text(&result["approvalPolicy"]);
    if policy.as_deref() != Some(APPROVAL_POLICY) {
        return Err(format!(
            "codex applied approval policy {policy:?}, expected {APPROVAL_POLICY}"
        ));
    }
    let sandbox = &result["sandbox"];
    let applied = sandbox
        .as_str()
        .or_else(|| sandbox["type"].as_str())
        .unwrap_or_default();
    if normalize(applied) != normalize(SANDBOX) {
        return Err(format!(
            "codex applied sandbox {applied:?}, expected {SANDBOX}"
        ));
    }
    let reviewer = result["approvalsReviewer"].as_str();
    if reviewer.is_some_and(|reviewer| reviewer != "user") {
        return Err(format!(
            "codex applied approvals reviewer {reviewer:?}, expected user"
        ));
    }
    Ok(())
}

// cost: time O(n), heap O(n), stack O(1), alloc 2
// vars: n = text 글자 수
// basis: estimate
/// `read-only`, `readOnly`, `read_only`를 같게 본다.
fn normalize(text: &str) -> String {
    text.chars()
        .filter(|c| !matches!(c, '-' | '_'))
        .collect::<String>()
        .to_ascii_lowercase()
}

/// 서버 준비 확인 한 번의 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum McpState {
    Ready,
    /// 아직 준비되지 않았거나 도구 목록을 받지 못한 서버와 이유.
    Waiting(String),
}

// cost: time O(s·e), heap O(1), stack O(1)
// vars: s = 기다릴 서버 수, e = 응답의 서버 수
// basis: estimate
/// `mcpServerStatus/list` 응답에서 기다릴 서버가 모두 `ready`이고 도구 목록 오류가 없는지 본다.
pub(super) fn mcp_state(result: &Value, servers: &[String]) -> McpState {
    let entries = result["data"].as_array().map_or(&[][..], Vec::as_slice);
    for server in servers {
        let Some(entry) = entries
            .iter()
            .find(|entry| entry["name"].as_str() == Some(server))
        else {
            return McpState::Waiting(format!("{server} is not listed"));
        };
        if !is_ready(&entry["runtimeStatus"]) {
            return McpState::Waiting(format!("{server} is {}", entry["runtimeStatus"]));
        }
        if !entry["toolsError"].is_null() {
            return McpState::Waiting(format!("{server} tools error {}", entry["toolsError"]));
        }
        if entry["tools"].is_null() {
            return McpState::Waiting(format!("{server} has no tool list"));
        }
    }
    McpState::Ready
}

fn is_ready(status: &Value) -> bool {
    let text = status
        .as_str()
        .or_else(|| status["status"].as_str())
        .or_else(|| status["state"].as_str())
        .or_else(|| status["type"].as_str());
    text.is_some_and(|text| text.eq_ignore_ascii_case("ready"))
}

// cost: time O(c), heap O(c), stack O(1), alloc 1
// vars: c = 항목의 변경 파일 수
// basis: estimate
/// `fileChange` 항목이 건드리는 경로.
pub(super) fn file_change_paths(item: &Value) -> Vec<String> {
    item["changes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|change| change["path"].as_str().map(str::to_owned))
        .collect()
}

// cost: time O(c + f), heap O(c + f), stack O(1), alloc 1
// vars: c = 명령 글자 수, f = 변경 파일 수
// basis: estimate
/// 승인 요청을 규칙이 읽는 호출로 바꾼다. 읽을 수 없는 요청(권한 요청, 명령이 없는 요청)은 `None`이고
/// 사용자에게 묻는다. `file_changes`는 `item/started`에서 받은 `fileChange` 항목의 경로다.
pub(super) fn call_of(
    method: &str,
    params: &Value,
    file_changes: &HashMap<String, Vec<String>>,
) -> Option<PermissionCall> {
    match method {
        "item/commandExecution/requestApproval" => shell(params["command"].as_str()?),
        "execCommandApproval" => {
            let words: Vec<&str> = params["command"]
                .as_array()?
                .iter()
                .filter_map(Value::as_str)
                .collect();
            shell(&words.join(" "))
        }
        "item/fileChange/requestApproval" => {
            let mut paths = file_changes
                .get(params["itemId"].as_str()?)
                .cloned()
                .unwrap_or_default();
            paths.extend(params["grantRoot"].as_str().map(str::to_owned));
            Some(edit(paths))
        }
        "applyPatchApproval" => {
            let changes = params["fileChanges"].as_object()?;
            Some(edit(changes.keys().cloned().collect()))
        }
        ELICITATION_METHOD => {
            let server = params["serverName"].as_str()?;
            let tool = mcp_tool(params).unwrap_or_else(|| UNKNOWN_MCP_TOOL.to_owned());
            Some(PermissionCall {
                tool: PermissionTool::Mcp,
                target: format!("mcp__{server}__{tool}"),
                paths: Vec::new(),
            })
        }
        _ => None,
    }
}

fn shell(command: &str) -> Option<PermissionCall> {
    Some(PermissionCall {
        tool: PermissionTool::Shell,
        target: unwrap_shell(command),
        paths: Vec::new(),
    })
}

fn edit(paths: Vec<String>) -> PermissionCall {
    PermissionCall {
        tool: PermissionTool::Edit,
        target: String::new(),
        paths,
    }
}

/// `_meta`에 도구 이름이 있으면 그것을, 없으면 요청 문구의 `tool "이름"`에서 읽는다.
fn mcp_tool(params: &Value) -> Option<String> {
    let meta = &params["_meta"];
    if let Some(name) = meta["tool_name"].as_str().or_else(|| meta["tool"].as_str()) {
        return Some(name.to_owned());
    }
    let message = params["message"].as_str()?;
    let after = message.split_once("tool \"")?.1;
    let name = after.split_once('"')?.0;
    (!name.is_empty()).then(|| name.to_owned())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn applied_policy_must_be_untrusted_read_only() {
        let applied = |policy: Value, sandbox: Value| {
            check_applied(&json!({ "approvalPolicy": policy, "sandbox": sandbox }))
        };

        assert!(applied(json!("untrusted"), json!({ "type": "readOnly" })).is_ok());
        assert!(applied(json!("untrusted"), json!("read-only")).is_ok());
        assert!(applied(json!("on-request"), json!({ "type": "readOnly" })).is_err());
        assert!(applied(json!("untrusted"), json!({ "type": "workspaceWrite" })).is_err());
        assert!(check_applied(&json!({ "approvalPolicy": "untrusted" })).is_err());
        assert!(
            check_applied(&json!({
                "approvalPolicy": "untrusted",
                "sandbox": "read-only",
                "approvalsReviewer": "guardian_subagent"
            }))
            .is_err()
        );
    }

    #[test]
    fn mcp_state_waits_for_every_listed_server() {
        let servers = vec!["docs".to_owned(), "web".to_owned()];
        let list = |web_status: &str| {
            json!({ "data": [
                { "name": "docs", "runtimeStatus": "ready", "tools": {}, "toolsError": null },
                { "name": "web", "runtimeStatus": web_status, "tools": [] },
            ] })
        };

        assert_eq!(mcp_state(&list("ready"), &servers), McpState::Ready);
        assert!(matches!(
            mcp_state(&list("starting"), &servers),
            McpState::Waiting(_)
        ));
        assert!(matches!(
            mcp_state(&json!({ "data": [] }), &servers),
            McpState::Waiting(_)
        ));
        assert_eq!(mcp_state(&json!({ "data": [] }), &[]), McpState::Ready);
        let broken = json!({ "data": [
            { "name": "docs", "runtimeStatus": "ready", "tools": [], "toolsError": "boom" },
        ] });
        assert!(matches!(
            mcp_state(&broken, &["docs".to_owned()]),
            McpState::Waiting(reason) if reason.contains("boom")
        ));
    }

    #[test]
    fn command_request_becomes_the_inner_shell_command() {
        let params = json!({ "command": "/bin/zsh -lc 'git status && ls'" });

        let call = call_of(
            "item/commandExecution/requestApproval",
            &params,
            &HashMap::new(),
        );

        assert_eq!(
            call,
            Some(PermissionCall {
                tool: PermissionTool::Shell,
                target: "git status && ls".to_owned(),
                paths: Vec::new(),
            })
        );
    }

    #[test]
    fn legacy_command_request_joins_the_words() {
        let params = json!({ "command": ["rm", "-rf", "build"] });

        let call = call_of("execCommandApproval", &params, &HashMap::new()).unwrap();

        assert_eq!(call.target, "rm -rf build");
    }

    #[test]
    fn file_change_request_uses_the_paths_seen_when_the_item_started() {
        let mut seen = HashMap::new();
        seen.insert(
            "item_f".to_owned(),
            vec!["a.rs".to_owned(), "b.rs".to_owned()],
        );

        let known = call_of(
            "item/fileChange/requestApproval",
            &json!({ "itemId": "item_f", "grantRoot": "/etc" }),
            &seen,
        )
        .unwrap();
        let unknown = call_of(
            "item/fileChange/requestApproval",
            &json!({ "itemId": "item_x" }),
            &seen,
        )
        .unwrap();

        assert_eq!(known.tool, PermissionTool::Edit);
        assert_eq!(known.paths, vec!["a.rs", "b.rs", "/etc"]);
        assert!(unknown.paths.is_empty());
    }

    #[test]
    fn patch_request_reads_the_changed_paths() {
        let params = json!({ "fileChanges": { "src/a.rs": {}, "src/b.rs": {} } });

        let call = call_of("applyPatchApproval", &params, &HashMap::new()).unwrap();

        assert_eq!(call.paths, vec!["src/a.rs", "src/b.rs"]);
    }

    #[test]
    fn mcp_request_names_the_server_and_the_tool() {
        let from_message = json!({
            "serverName": "docs",
            "message": "Allow the docs MCP server to run tool \"search\"?",
        });
        let from_meta = json!({ "serverName": "docs", "_meta": { "tool_name": "read" } });
        let unknown = json!({ "serverName": "docs", "message": "ok?" });

        let target = |params: &Value| {
            call_of(ELICITATION_METHOD, params, &HashMap::new())
                .unwrap()
                .target
        };

        assert_eq!(target(&from_message), "mcp__docs__search");
        assert_eq!(target(&from_meta), "mcp__docs__read");
        assert_eq!(target(&unknown), "mcp__docs__?");
    }

    #[test]
    fn requests_rules_cannot_read_have_no_call() {
        let none = |method: &str, params: Value| call_of(method, &params, &HashMap::new());

        assert_eq!(none("item/permissions/requestApproval", json!({})), None);
        assert_eq!(
            none("item/commandExecution/requestApproval", json!({})),
            None
        );
        assert_eq!(none(ELICITATION_METHOD, json!({})), None);
    }
}
