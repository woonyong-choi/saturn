//! Codex 권한 연결: 승인 요청을 규칙이 읽는 호출로 옮기고, 시작 때 적용 정책과 MCP 준비를 확인한다.
//! 설계: docs/design/permissions.md#codex-구성

use std::collections::HashMap;

use saturn_protocol::event::{PermissionCall, PermissionTool};
use serde_json::Value;

use crate::providers::tool_detail::unwrap_shell;

/// 모든 요청을 Saturn에 올리려고 `thread/start`에 주는 승인 정책. 설정 키로는 쓸 수 없다.
pub(super) const APPROVAL_POLICY: &str = "untrusted";

/// 허가한 명령이 작업 폴더(와 더한 폴더)에 쓸 수 있게 주는 샌드박스. 읽기 전용이면 허가한 `cargo build`도 `Cargo.lock`을 쓰지 못한다.
/// 파일 편집은 승인 정책 `untrusted` 때문에 이 샌드박스에서도 승인 요청으로 오고, 실행 전에 Saturn 규칙이 판정한다.
pub(super) const SANDBOX: &str = "workspace-write";

const ELICITATION_METHOD: &str = "mcpServer/elicitation/request";

/// 도구 이름을 알 수 없는 MCP 요청의 도구 자리.
const UNKNOWN_MCP_TOOL: &str = "?";

/// `thread/start`와 `thread/resume`의 응답이 실제로 적용한 승인 정책, 샌드박스, 검토자가 기대와 다르면 이유 한 줄.
pub(super) fn check_applied(result: &Value) -> Result<(), String> {
    let policy = super::value_text(&result["approvalPolicy"]);
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
    if sandbox["networkAccess"].as_bool() == Some(true) {
        return Err("codex applied a sandbox with network access".to_owned());
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

/// 서버 준비 확인 한 번의 결과. 서버마다 준비, 기다림, 쓸 수 없음 셋 중 하나로 나눈다.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct McpCheck {
    /// 아직 준비됐는지 알 수 없어 다시 물을 서버와 이유.
    pub(super) waiting: Vec<String>,
    /// 시작에 실패해 도구를 쓸 수 없는 서버와 이유.
    pub(super) unavailable: Vec<String>,
}

/// 서버 하나의 판정.
enum ServerState {
    Ready,
    Waiting(String),
    Unavailable(String),
}

// cost: time O(s·e), heap O(s), stack O(1), alloc s
// vars: s = 확인할 서버 수, e = 응답의 서버 수
// basis: estimate
/// `mcpServerStatus/list` 응답을 서버별로 판정한다. codex-cli 0.158.0은 `runtimeStatus`를 `null`로 주고
/// 시작이 끝난 서버만 목록에 낸다. 그때 준비된 서버는 `serverInfo`가 있고 `toolsError`가 없으며, 시작에 실패한 서버는
/// `toolsError`가 차 있다(빈 `tools`가 함께 온다). `runtimeStatus`가 문자열로 오는 버전은 그 값을 먼저 본다.
pub(super) fn mcp_check(result: &Value, servers: &[String]) -> McpCheck {
    let entries = result["data"].as_array().map_or(&[][..], Vec::as_slice);
    let mut check = McpCheck::default();
    for server in servers {
        let state = match entries
            .iter()
            .find(|entry| entry["name"].as_str() == Some(server))
        {
            Some(entry) => server_state(server, entry),
            None => ServerState::Waiting(format!("{server} is not listed")),
        };
        match state {
            ServerState::Ready => {}
            ServerState::Waiting(reason) => check.waiting.push(reason),
            ServerState::Unavailable(reason) => check.unavailable.push(reason),
        }
    }
    check
}

fn server_state(server: &str, entry: &Value) -> ServerState {
    let failed = || {
        ServerState::Unavailable(format!(
            "{server} failed to start: {}",
            value_line(&entry["toolsError"])
        ))
    };
    match runtime_status(&entry["runtimeStatus"]) {
        Some(status) if status.eq_ignore_ascii_case("ready") => {
            if !entry["toolsError"].is_null() {
                failed()
            } else if entry["tools"].is_null() {
                ServerState::Waiting(format!("{server} has no tool list"))
            } else {
                ServerState::Ready
            }
        }
        Some(status)
            if status.eq_ignore_ascii_case("failed")
                || status.eq_ignore_ascii_case("cancelled") =>
        {
            ServerState::Unavailable(format!("{server} is {status}"))
        }
        Some(status) => ServerState::Waiting(format!("{server} is {status}")),
        None if !entry["toolsError"].is_null() => failed(),
        None if !entry["serverInfo"].is_null() => ServerState::Ready,
        None => ServerState::Waiting(format!("{server} has no server info")),
    }
}

fn runtime_status(status: &Value) -> Option<&str> {
    status
        .as_str()
        .or_else(|| status["status"].as_str())
        .or_else(|| status["state"].as_str())
        .or_else(|| status["type"].as_str())
}

/// 오류 값을 한 줄 글로.
fn value_line(value: &Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_owned)
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
        "item/commandExecution/requestApproval" => shell(
            params["command"].as_str()?,
            runs_outside_sandbox(params),
            only_reads(params),
        ),
        "execCommandApproval" => {
            let words: Vec<&str> = params["command"]
                .as_array()?
                .iter()
                .filter_map(Value::as_str)
                .collect();
            shell(&words.join(" "), runs_outside_sandbox(params), None)
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
                outside_sandbox: false,
                reads_only: false,
            })
        }
        _ => None,
    }
}

/// `reads`는 provider가 읽기만 한다고 분류한 명령이 읽는 경로이고 분류가 없으면 `None`.
fn shell(
    command: &str,
    outside_sandbox: bool,
    reads: Option<Vec<String>>,
) -> Option<PermissionCall> {
    Some(PermissionCall {
        tool: PermissionTool::Shell,
        target: unwrap_shell(command),
        reads_only: reads.is_some(),
        paths: reads.unwrap_or_default(),
        outside_sandbox,
    })
}

// cost: time O(1), heap O(1), stack O(1), alloc 0
// vars: 없음
// basis: estimate
/// 명령 승인 요청이 샌드박스 밖 실행을 원하는지. 요청에는 실행 범위를 직접 알리는 값이 없고, 샌드박스 밖 실행을 요청하는
/// 도구 호출(`sandbox_permissions=require_escalated`)만 이유(`reason`)를 붙여 온다. 이유, 추가 권한, 네트워크 승인
/// 맥락 중 하나라도 있으면 샌드박스 밖 실행으로 본다. 범위를 확신할 수 없는 쪽을 밖으로 보므로 자동 허용하지 않는다.
pub(super) fn runs_outside_sandbox(params: &Value) -> bool {
    params["reason"]
        .as_str()
        .is_some_and(|reason| !reason.trim().is_empty())
        || !params["additionalPermissions"].is_null()
        || !params["networkApprovalContext"].is_null()
}

/// Codex가 명령을 분석해 파일 읽기, 목록, 검색(`commandActions`의 `read`, `listFiles`, `search`)만 하는 것으로 분류했으면
/// 읽는 경로(동작의 `path`)를 돌려준다. 한 동작이라도 그 밖의 종류(`unknown` 포함)이거나 분류가 없으면 `None`이다.
fn only_reads(params: &Value) -> Option<Vec<String>> {
    let actions = params["commandActions"].as_array()?;
    let is_read = |action: &Value| {
        matches!(
            action["type"].as_str(),
            Some("read" | "listFiles" | "search")
        )
    };
    if actions.is_empty() || !actions.iter().all(is_read) {
        return None;
    }
    Some(
        actions
            .iter()
            .filter_map(|action| action["path"].as_str().map(str::to_owned))
            .collect(),
    )
}

fn edit(paths: Vec<String>) -> PermissionCall {
    PermissionCall {
        tool: PermissionTool::Edit,
        target: String::new(),
        paths,
        outside_sandbox: false,
        reads_only: false,
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
    fn applied_policy_must_be_untrusted_workspace_write_without_network() {
        let applied = |policy: Value, sandbox: Value| {
            check_applied(&json!({ "approvalPolicy": policy, "sandbox": sandbox }))
        };

        assert!(applied(json!("untrusted"), json!({ "type": "workspaceWrite" })).is_ok());
        assert!(applied(json!("untrusted"), json!("workspace-write")).is_ok());
        assert!(
            applied(
                json!("untrusted"),
                json!({ "type": "workspaceWrite", "networkAccess": false })
            )
            .is_ok()
        );
        assert!(
            applied(
                json!("untrusted"),
                json!({ "type": "workspaceWrite", "networkAccess": true })
            )
            .is_err()
        );
        assert!(applied(json!("on-request"), json!({ "type": "workspaceWrite" })).is_err());
        assert!(applied(json!("untrusted"), json!({ "type": "readOnly" })).is_err());
        assert!(applied(json!("untrusted"), json!({ "type": "dangerFullAccess" })).is_err());
        assert!(check_applied(&json!({ "approvalPolicy": "untrusted" })).is_err());
        assert!(
            check_applied(&json!({
                "approvalPolicy": "untrusted",
                "sandbox": "workspace-write",
                "approvalsReviewer": "guardian_subagent"
            }))
            .is_err()
        );
    }

    /// codex-cli 0.158.0 `mcpServerStatus/list` 실측 모양. `runtimeStatus`는 항상 `null`이다.
    fn measured_list() -> Value {
        json!({ "data": [
            { "name": "bad", "runtimeStatus": null, "serverInfo": null, "serverCapabilities": null,
              "tools": {}, "toolsError": "MCP startup failed: No such file or directory (os error 2)",
              "resources": [], "resourceTemplates": [], "authStatus": "unsupported" },
            { "name": "good", "runtimeStatus": null,
              "serverInfo": { "name": "good", "title": null, "version": "1" },
              "serverCapabilities": { "tools": {} },
              "tools": { "echo": { "name": "echo", "description": "e", "inputSchema": { "type": "object" } } },
              "toolsError": null, "resources": [], "resourceTemplates": [], "authStatus": "unsupported" },
            { "name": "quiet", "runtimeStatus": null,
              "serverInfo": { "name": "quiet", "title": null, "version": "1" },
              "tools": {}, "toolsError": null },
        ], "nextCursor": null })
    }

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn mcp_check_sorts_each_server_into_ready_waiting_or_unavailable() {
        let status_list = |status: &str| {
            json!({ "data": [
                { "name": "docs", "runtimeStatus": status, "tools": {}, "toolsError": null },
            ] })
        };
        let silent = json!({ "data": [
            { "name": "docs", "runtimeStatus": null, "serverInfo": null, "tools": {}, "toolsError": null },
        ] });
        let broken = json!({ "data": [
            { "name": "docs", "runtimeStatus": "ready", "tools": [], "toolsError": "boom" },
        ] });
        let empty = json!({ "data": [] });
        // (이름, 서버 응답, 대상 서버, 기다리는 수, 쓸 수 없는 서버 메시지에 든 글)
        let cases = [
            (
                "null runtime status with server info is ready",
                measured_list(),
                names(&["good", "quiet"]),
                0,
                vec![],
            ),
            (
                "tools error is unavailable, not waiting",
                measured_list(),
                names(&["bad", "good"]),
                0,
                vec!["No such file"],
            ),
            (
                "no server info is waiting",
                silent,
                names(&["docs"]),
                1,
                vec![],
            ),
            (
                "missing entry is waiting",
                empty.clone(),
                names(&["docs"]),
                1,
                vec![],
            ),
            ("no servers asked for is ready", empty, vec![], 0, vec![]),
            (
                "string status ready",
                status_list("ready"),
                names(&["docs"]),
                0,
                vec![],
            ),
            (
                "string status starting",
                status_list("starting"),
                names(&["docs"]),
                1,
                vec![],
            ),
            (
                "string status failed",
                status_list("failed"),
                names(&["docs"]),
                0,
                vec![""],
            ),
            (
                "tools error beats a ready string status",
                broken,
                names(&["docs"]),
                0,
                vec!["boom"],
            ),
        ];
        for (name, list, servers, waiting, unavailable) in cases {
            let check = mcp_check(&list, &servers);

            assert_eq!(check.waiting.len(), waiting, "{name}");
            assert_eq!(check.unavailable.len(), unavailable.len(), "{name}");
            for (message, fragment) in check.unavailable.iter().zip(&unavailable) {
                assert!(message.contains(fragment), "{name}: {message}");
            }
            if waiting == 0 && unavailable.is_empty() {
                assert_eq!(check, McpCheck::default(), "{name}");
            }
        }
        let failed = mcp_check(&measured_list(), &names(&["bad", "good"]));
        assert!(failed.unavailable[0].starts_with("bad failed to start"));
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
                outside_sandbox: false,
                reads_only: false,
            })
        );
    }

    /// 실측: 샌드박스 밖 실행을 요청한 호출은 `reason`이 붙어 오고, 일반 승인 요청에는 `reason`이 없다.
    #[test]
    fn command_request_with_a_reason_or_extra_permissions_runs_outside_the_sandbox() {
        let outside = |params: Value| {
            call_of(
                "item/commandExecution/requestApproval",
                &params,
                &HashMap::new(),
            )
            .unwrap()
            .outside_sandbox
        };

        assert!(!outside(json!({ "command": "cargo build" })));
        assert!(!outside(
            json!({ "command": "cargo build", "reason": null })
        ));
        assert!(!outside(json!({ "command": "cargo build", "reason": " " })));
        assert!(outside(
            json!({ "command": "security find-generic-password -w", "reason": "needs the keychain" })
        ));
        assert!(outside(
            json!({ "command": "ls", "additionalPermissions": { "fileSystem": {} } })
        ));
        assert!(outside(
            json!({ "command": "curl x", "networkApprovalContext": { "host": "x" } })
        ));
    }

    /// 실측: `sed -n 1,240p 파일`은 `read`, `sleep 6; echo done`은 `unknown`으로 분류돼 온다.
    #[test]
    fn command_actions_that_only_read_mark_the_request_as_reads_only() {
        let reads_only = |actions: Value| {
            call_of(
                "item/commandExecution/requestApproval",
                &json!({ "command": "x", "commandActions": actions }),
                &HashMap::new(),
            )
            .unwrap()
            .reads_only
        };

        assert!(reads_only(
            json!([{ "type": "read", "path": "src/main.rs" }])
        ));
        assert!(reads_only(
            json!([{ "type": "listFiles" }, { "type": "search" }])
        ));
        assert!(!reads_only(json!([{ "type": "unknown" }])));
        assert!(!reads_only(
            json!([{ "type": "read" }, { "type": "unknown" }])
        ));
        assert!(!reads_only(json!([])));
        assert!(!reads_only(Value::Null));
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
