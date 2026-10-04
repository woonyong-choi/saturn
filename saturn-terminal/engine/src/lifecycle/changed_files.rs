//! 수정 파일 목록 테스트: 실행 경계의 폴더 상태 차이로 세고, provider 이벤트는 수정 주체에만 쓴다.
//! 설계: docs/design/providers-and-sessions.md#수정-파일-목록

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use saturn_core::sessions::changes::{ChangeKind, ChangeSet};
use saturn_protocol::event::{Activity, ProviderEvent, ToolCategory, ToolDetail};
use saturn_protocol::ids::AgentId;

use super::support::{Flow, idle_reply, turn_completed};
use crate::providers::test_support::Call;

fn workdir(flow: &Flow) -> PathBuf {
    flow.fixture.workdir.canonicalize().unwrap()
}

fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, text).unwrap();
    path
}

fn init_git(dir: &Path) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["init", "-q"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
}

fn edit_event(agent: AgentId, path: &Path) -> ProviderEvent {
    ProviderEvent::ToolCall {
        agent,
        subagent: None,
        call_id: "edit-1".to_owned(),
        activity: Activity::EditingFile,
        detail: ToolDetail {
            category: ToolCategory::FileEdit,
            paths: vec![path.to_string_lossy().into_owned()],
            read_lines: None,
            changed: None,
        },
    }
}

/// 마지막으로 측정한 실행의 목록. 경로는 작업 폴더 기준으로 줄이고 주체와 함께 돌려준다.
async fn last_changes(flow: &Flow) -> (Vec<(String, ChangeKind, Vec<String>)>, bool) {
    let runs = flow.engine.store.run_changes(flow.chat).await.unwrap();
    let set: &ChangeSet = &runs.last().expect("a measured run").set;
    let base = workdir(flow);
    let files = set
        .files
        .iter()
        .map(|change| {
            let name = Path::new(&change.path).strip_prefix(&base).unwrap();
            (
                name.to_string_lossy().into_owned(),
                change.kind,
                change.actors.clone(),
            )
        })
        .collect();
    (files, set.is_partial)
}

#[tokio::test]
async fn shell_edits_without_provider_events_are_listed_at_tree_idle() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    let base = workdir(&flow);
    write(&base, "keep.txt", "same");
    write(&base, "edit.txt", "one");
    flow.submit("fix the build").await;
    let agent = flow.agent();

    write(&base, "edit.txt", "one two");
    write(&base, "sub/new.txt", "made by a shell command");
    write(&base, "target/out.bin", "build output is ignored");
    flow.claude_event(turn_completed(agent)).await;

    let (files, is_partial) = last_changes(&flow).await;
    assert_eq!(
        files,
        vec![
            ("edit.txt".to_owned(), ChangeKind::Modified, Vec::new()),
            ("sub/new.txt".to_owned(), ChangeKind::Added, Vec::new()),
        ]
    );
    assert!(!is_partial);
}

#[tokio::test]
async fn git_repository_lists_shell_edits_and_skips_ignored_files() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    let base = workdir(&flow);
    init_git(&base);
    write(&base, ".gitignore", "*.log\n");
    flow.submit("fix the build").await;
    let agent = flow.agent();

    write(&base, "src/lib.rs", "fn main() {}");
    write(&base, "debug.log", "ignored by .gitignore");
    flow.claude_event(turn_completed(agent)).await;

    let (files, _) = last_changes(&flow).await;
    assert_eq!(
        files,
        vec![("src/lib.rs".to_owned(), ChangeKind::Added, Vec::new())]
    );
}

#[tokio::test]
async fn provider_events_only_name_who_edited() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    let base = workdir(&flow);
    flow.submit("fix the build").await;
    let agent = flow.agent();

    let reported = write(&base, "reported.rs", "edited through the edit tool");
    write(&base, "shell.rs", "edited by a shell command");
    flow.claude_event(edit_event(agent, &reported)).await;
    flow.claude_event(edit_event(agent, &base.join("claimed-but-untouched.rs")))
        .await;
    flow.claude_event(turn_completed(agent)).await;

    let (files, _) = last_changes(&flow).await;
    assert_eq!(
        files,
        vec![
            (
                "reported.rs".to_owned(),
                ChangeKind::Added,
                vec!["main agent".to_owned()]
            ),
            ("shell.rs".to_owned(), ChangeKind::Added, Vec::new()),
        ]
    );
}

#[tokio::test]
async fn too_large_folder_is_cut_at_the_limit_and_marked_partial() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.engine.flow.change_limits.max_files = 2;
    let base = workdir(&flow);
    for index in 0..4 {
        write(&base, &format!("seed{index}.txt"), "x");
    }
    flow.submit("fix the build").await;
    let agent = flow.agent();

    write(&base, "seed0.txt", "changed");
    flow.claude_event(turn_completed(agent)).await;

    let (files, is_partial) = last_changes(&flow).await;
    assert_eq!(
        files,
        vec![("seed0.txt".to_owned(), ChangeKind::Modified, Vec::new())]
    );
    assert!(is_partial);
}

#[tokio::test]
async fn stop_confirmation_input_lists_the_files_changed_before_the_stop() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    let base = workdir(&flow);
    flow.submit("fix the build").await;
    let agent = flow.agent();

    write(&base, "half-done.rs", "partly written by a child process");
    flow.engine.stop_chat(flow.chat).await.unwrap();
    flow.claude_event(turn_completed(agent)).await;
    flow.engine.continue_held(flow.chat, None).await.unwrap();
    flow.settle().await;

    let sent: Vec<String> = flow
        .fake
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::SendTurn { text, .. } => Some(text),
            _ => None,
        })
        .collect();
    let confirmation = sent.last().unwrap();
    let listed = base.join("half-done.rs");
    assert!(
        confirmation.contains(&format!(
            "Files changed during that turn: {} (added, by shell or child process)",
            listed.display()
        )),
        "{confirmation}"
    );
    assert!(confirmation.ends_with("Request:\nfix the build"));
}

#[tokio::test]
async fn confirmation_without_changes_has_no_file_line() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let agent = flow.agent();

    flow.engine.stop_chat(flow.chat).await.unwrap();
    flow.claude_event(turn_completed(agent)).await;
    flow.engine.continue_held(flow.chat, None).await.unwrap();
    flow.settle().await;

    let sent = flow.fake.calls().into_iter().any(
        |call| matches!(call, Call::SendTurn { text, .. } if text.contains("Files changed during")),
    );
    assert!(!sent);
}
