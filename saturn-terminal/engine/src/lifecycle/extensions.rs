//! 확장 저장소 테스트: 설치는 원본을 확장 저장소에 두고 부분별 판정을 기록에 남기며 결과를 대화 기록에 한 줄로 알린다.
//! 설계: docs/design/extensions.md#설치

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use saturn_protocol::envelope::ServerMessage;
use saturn_protocol::ids::Provider;
use saturn_protocol::rpc::{
    ChatNotice, ExtensionInfo, ExtensionPartKind, Injectability, Notification, QueryResult, Request,
};

use super::support::{CLIENT, Flow};
use super::*;
use crate::providers::test_support::{FakeAdapter, FakeProvider, fake_descriptor};
use crate::providers::{Adapter, BoxFuture, Feature, LaunchSpec, ProviderConnection};
use crate::rpc::ClientId;

/// 스킬과 MCP 서버만 주입하는 어댑터.
const NARROW: Provider = Provider::from_static("narrow-agent");
/// 네 종류를 모두 주입하는 어댑터.
const WIDE: Provider = Provider::from_static("wide-agent");

const NARROW_FEATURES: &[Feature] = &[
    Feature::Inject(ExtensionPartKind::Skill),
    Feature::Inject(ExtensionPartKind::McpServer),
];
const WIDE_FEATURES: &[Feature] = &[
    Feature::Inject(ExtensionPartKind::Skill),
    Feature::Inject(ExtensionPartKind::McpServer),
    Feature::Inject(ExtensionPartKind::Command),
    Feature::Inject(ExtensionPartKind::Hook),
];

fn register(flow: &mut Flow, id: Provider, features: &'static [Feature]) {
    let mut descriptor = fake_descriptor(id);
    descriptor.features = features;
    flow.engine
        .registry
        .register(Arc::new(FakeAdapter {
            descriptor,
            provider: FakeProvider::new(id),
        }))
        .unwrap();
}

async fn flow_with_two_adapters() -> Flow {
    let mut flow = Flow::new(Vec::new()).await;
    register(&mut flow, NARROW, NARROW_FEATURES);
    register(&mut flow, WIDE, WIDE_FEATURES);
    flow
}

fn write(root: &Path, file: &str, content: &str) {
    let path = root.join(file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

/// 스킬 하나, 명령 하나, MCP 서버 하나, 훅 하나로 이루어진 묶음 폴더.
fn bundle(flow: &Flow, name: &str) -> PathBuf {
    let root = flow.fixture.root.path().join("sources").join(name);
    write(&root, "skills/commit-helper/SKILL.md", "# commit helper");
    write(&root, "commands/review.md", "review the diff");
    write(
        &root,
        ".mcp.json",
        r#"{"mcpServers":{"lint":{"command":"lint"}}}"#,
    );
    write(&root, "hooks/hooks.json", r#"{"hooks":{"PreToolUse":[]}}"#);
    root
}

fn store_dir(flow: &Flow) -> PathBuf {
    flow.fixture.options.home.join("extensions")
}

/// 알림 중 확장 알림만 모은다.
fn extension_notices(notifications: &[Notification]) -> Vec<ChatNotice> {
    notifications
        .iter()
        .filter_map(|notification| match notification {
            Notification::ChatNotice { notice, .. }
                if matches!(
                    notice,
                    ChatNotice::ExtensionInstalled { .. }
                        | ChatNotice::ExtensionRemoved { .. }
                        | ChatNotice::ExtensionFailed { .. }
                ) =>
            {
                Some(notice.clone())
            }
            _ => None,
        })
        .collect()
}

fn installed(notice: &ChatNotice) -> &ExtensionInfo {
    match notice {
        ChatNotice::ExtensionInstalled { extension } => extension,
        other => panic!("expected an install notice, got {other:?}"),
    }
}

/// 부분의 두 가짜 어댑터 판정. 나머지 어댑터의 판정은 보지 않는다.
fn verdicts(
    info: &ExtensionInfo,
    kind: ExtensionPartKind,
    name: &str,
) -> Vec<(String, Injectability)> {
    let part = info
        .parts
        .iter()
        .find(|part| part.kind == kind && part.name == name)
        .unwrap_or_else(|| panic!("part {name} should exist"));
    part.verdicts
        .iter()
        .filter(|(provider, _)| *provider == NARROW || *provider == WIDE)
        .map(|(provider, verdict)| (provider.to_string(), *verdict))
        .collect()
}

/// `ListExtensions` 요청을 소켓으로 보내고 답으로 온 목록을 돌려준다.
async fn list(flow: &mut Flow, client: &mut Client) -> Vec<ExtensionInfo> {
    drive(&mut flow.engine, async {
        let QueryResult::ExtensionList { extensions } =
            client.query(9, Request::ListExtensions).await
        else {
            panic!("expected the extension list");
        };
        extensions
    })
    .await
}

/// 내려받기처럼 별도 작업이 끝나야 오는 확장 알림 하나가 올 때까지 요청 처리를 돌린다.
async fn window_driven(flow: &mut Flow, client: &mut Client) -> Vec<Notification> {
    drive(&mut flow.engine, async {
        loop {
            let message = client.recv().await;
            if let ServerMessage::Notification(message) = message
                && !extension_notices(std::slice::from_ref(&message.notification)).is_empty()
            {
                return vec![message.notification];
            }
        }
    })
    .await
}

async fn install(flow: &mut Flow, source: &str) {
    flow.engine
        .install_extension(CLIENT, flow.chat, source)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_folder_is_copied_to_the_store_and_each_part_is_judged_per_provider() {
    let mut flow = flow_with_two_adapters().await;
    let source = bundle(&flow, "review-kit");
    write(&source, ".git/config", "not part of the original");
    let outside = flow.fixture.root.path().join("outside.txt");
    std::fs::write(&outside, "secret").unwrap();
    std::os::unix::fs::symlink(&outside, source.join("linked.txt")).unwrap();
    let mut client = flow.client().await;

    install(&mut flow, &source.display().to_string()).await;

    let notices = extension_notices(&client.window().await);
    assert_eq!(notices.len(), 1);
    let info = installed(&notices[0]);
    assert_eq!(info.name, "review-kit");
    assert_eq!(info.source, source.display().to_string());
    let narrow = |verdict| ("narrow-agent".to_owned(), verdict);
    let wide = |verdict| ("wide-agent".to_owned(), verdict);
    assert_eq!(
        verdicts(info, ExtensionPartKind::Skill, "commit-helper"),
        vec![
            narrow(Injectability::Injectable),
            wide(Injectability::Injectable)
        ]
    );
    assert_eq!(
        verdicts(info, ExtensionPartKind::McpServer, "lint"),
        vec![
            narrow(Injectability::Injectable),
            wide(Injectability::Injectable)
        ]
    );
    assert_eq!(
        verdicts(info, ExtensionPartKind::Command, "review"),
        vec![
            narrow(Injectability::Unavailable),
            wide(Injectability::Injectable)
        ]
    );
    assert_eq!(
        verdicts(info, ExtensionPartKind::Hook, "PreToolUse"),
        vec![
            narrow(Injectability::Unavailable),
            wide(Injectability::Injectable)
        ]
    );
    let kept = store_dir(&flow).join("review-kit");
    assert_eq!(
        std::fs::read_to_string(kept.join("skills/commit-helper/SKILL.md")).unwrap(),
        "# commit helper"
    );
    assert!(!kept.join(".git").exists());
    assert!(!kept.join("linked.txt").exists());
    let rows = flow.engine.store.extension_rows().await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "review-kit");
}

#[tokio::test]
async fn a_folder_with_a_skill_file_at_its_root_is_one_skill_named_after_the_folder() {
    let mut flow = flow_with_two_adapters().await;
    let source = flow.fixture.root.path().join("sources/commit-helper");
    write(&source, "SKILL.md", "# commit helper");
    let mut client = flow.client().await;

    install(&mut flow, &source.display().to_string()).await;

    let notices = extension_notices(&client.window().await);
    let info = installed(&notices[0]);
    assert_eq!(info.parts.len(), 1);
    assert_eq!(
        (info.parts[0].kind, info.parts[0].name.as_str()),
        (ExtensionPartKind::Skill, "commit-helper")
    );
}

#[tokio::test]
async fn a_git_address_is_cloned_by_the_engine_without_the_git_folder() {
    let mut flow = flow_with_two_adapters().await;
    let repo = flow.fixture.root.path().join("remotes/review-kit.git");
    write(&repo, "skills/commit-helper/SKILL.md", "# commit helper");
    for args in [
        vec!["init", "--quiet"],
        vec!["add", "."],
        vec![
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "commit",
            "--quiet",
            "-m",
            "x",
        ],
    ] {
        let status = std::process::Command::new("git")
            .args(&args)
            .current_dir(&repo)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} should succeed");
    }
    let mut client = flow.client().await;

    install(&mut flow, &format!("file://{}", repo.display())).await;

    let notices = extension_notices(&window_driven(&mut flow, &mut client).await);
    assert_eq!(installed(&notices[0]).name, "review-kit");
    let kept = store_dir(&flow).join("review-kit");
    assert!(kept.join("skills/commit-helper/SKILL.md").is_file());
    assert!(!kept.join(".git").exists());
}

#[tokio::test]
async fn a_failed_clone_leaves_nothing_behind() {
    let mut flow = flow_with_two_adapters().await;
    let missing = flow.fixture.root.path().join("remotes/missing.git");
    let mut client = flow.client().await;

    install(&mut flow, &format!("file://{}", missing.display())).await;

    let notices = extension_notices(&window_driven(&mut flow, &mut client).await);
    assert!(matches!(
        &notices[0],
        ChatNotice::ExtensionFailed { name: Some(name), .. } if name == "missing"
    ));
    assert!(!store_dir(&flow).join("missing").exists());
    assert!(flow.engine.store.extension_rows().await.unwrap().is_empty());
}

#[tokio::test]
async fn an_install_that_cannot_proceed_says_why_and_keeps_the_earlier_install() {
    let mut flow = flow_with_two_adapters().await;
    let source = bundle(&flow, "review-kit");
    install(&mut flow, &source.display().to_string()).await;
    write(&source, "skills/commit-helper/SKILL.md", "# changed");
    let empty = flow.fixture.root.path().join("sources/empty-kit");
    std::fs::create_dir_all(&empty).unwrap();
    let broken = flow.fixture.root.path().join("sources/broken-kit");
    write(&broken, ".mcp.json", "{ not json");
    let mut client = flow.client().await;

    for source in [
        source.display().to_string(),
        empty.display().to_string(),
        broken.display().to_string(),
        "relative/kit".to_owned(),
        flow.fixture
            .root
            .path()
            .join("missing")
            .display()
            .to_string(),
    ] {
        install(&mut flow, &source).await;
    }

    let notices = extension_notices(&client.window().await);
    assert_eq!(notices.len(), 5);
    assert!(
        notices
            .iter()
            .all(|notice| matches!(notice, ChatNotice::ExtensionFailed { .. }))
    );
    let store = store_dir(&flow);
    assert_eq!(
        std::fs::read_to_string(store.join("review-kit/skills/commit-helper/SKILL.md")).unwrap(),
        "# commit helper"
    );
    assert!(!store.join("empty-kit").exists());
    assert!(!store.join("broken-kit").exists());
    assert_eq!(flow.engine.store.extension_rows().await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_folder_named_like_a_hidden_file_is_refused() {
    let mut flow = flow_with_two_adapters().await;
    let hidden = flow.fixture.root.path().join("sources/.hidden-kit");
    write(&hidden, "SKILL.md", "# hidden");
    let mut client = flow.client().await;

    install(&mut flow, &hidden.display().to_string()).await;

    let notices = extension_notices(&client.window().await);
    assert!(matches!(
        &notices[0],
        ChatNotice::ExtensionFailed { reason, .. } if reason.contains("invalid extension name")
    ));
    assert!(flow.engine.store.extension_rows().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_source_that_contains_the_store_is_refused() {
    let mut flow = flow_with_two_adapters().await;
    let home = flow.fixture.options.home.clone();
    write(
        &home,
        "SKILL.md",
        "# a skill in the folder that holds the store",
    );
    std::fs::create_dir_all(store_dir(&flow)).unwrap();
    let mut client = flow.client().await;

    install(&mut flow, &home.display().to_string()).await;

    let notices = extension_notices(&client.window().await);
    assert!(matches!(
        &notices[0],
        ChatNotice::ExtensionFailed { reason, .. } if reason.contains("extension store")
    ));
    assert!(!store_dir(&flow).join("home").exists());
}

#[tokio::test]
async fn install_from_a_client_not_attached_to_the_chat_is_refused() {
    let mut flow = flow_with_two_adapters().await;
    let source = bundle(&flow, "review-kit");

    let result = flow
        .engine
        .install_extension(ClientId(2), flow.chat, &source.display().to_string())
        .await;

    assert!(matches!(result, Err(EngineError::ChatNotAttached { .. })));
    assert!(flow.engine.store.extension_rows().await.unwrap().is_empty());
}

#[tokio::test]
async fn remove_deletes_the_original_and_the_row_and_refuses_names_outside_the_store() {
    let mut flow = flow_with_two_adapters().await;
    let source = bundle(&flow, "review-kit");
    install(&mut flow, &source.display().to_string()).await;
    let outside = flow.fixture.root.path().join("keep-me");
    std::fs::create_dir_all(&outside).unwrap();
    let mut client = flow.client().await;

    for name in ["../../keep-me", "never-installed", "review-kit"] {
        flow.engine
            .remove_extension(CLIENT, flow.chat, name)
            .await
            .unwrap();
    }

    let notices = extension_notices(&client.window().await);
    assert!(matches!(&notices[0], ChatNotice::ExtensionFailed { .. }));
    assert!(matches!(&notices[1], ChatNotice::ExtensionFailed { .. }));
    assert_eq!(
        notices[2],
        ChatNotice::ExtensionRemoved {
            name: "review-kit".to_owned()
        }
    );
    assert!(outside.exists());
    assert!(!store_dir(&flow).join("review-kit").exists());
    assert!(flow.engine.store.extension_rows().await.unwrap().is_empty());
}

#[tokio::test]
async fn remove_clears_the_row_even_when_the_original_is_already_gone() {
    let mut flow = flow_with_two_adapters().await;
    let source = bundle(&flow, "review-kit");
    install(&mut flow, &source.display().to_string()).await;
    std::fs::remove_dir_all(store_dir(&flow).join("review-kit")).unwrap();
    let mut client = flow.client().await;

    flow.engine
        .remove_extension(CLIENT, flow.chat, "review-kit")
        .await
        .unwrap();

    assert_eq!(
        extension_notices(&client.window().await),
        vec![ChatNotice::ExtensionRemoved {
            name: "review-kit".to_owned()
        }]
    );
    assert!(flow.engine.store.extension_rows().await.unwrap().is_empty());
}

#[tokio::test]
async fn list_answers_the_requesting_client_in_install_order() {
    let mut flow = flow_with_two_adapters().await;
    for name in ["second-kit", "first-kit"] {
        let source = bundle(&flow, name);
        install(&mut flow, &source.display().to_string()).await;
    }
    let mut client = flow.client().await;

    let listed = list(&mut flow, &mut client).await;
    assert_eq!(
        listed
            .iter()
            .map(|info| info.name.as_str())
            .collect::<Vec<_>>(),
        ["second-kit", "first-kit"]
    );
}

/// `Unknown`을 돌려주다가 값을 바꾸면 달라지는 어댑터.
#[derive(Debug)]
struct Moody {
    inner: FakeAdapter,
    answer: Arc<Mutex<Injectability>>,
}

impl Adapter for Moody {
    fn descriptor(&self) -> &crate::providers::Descriptor {
        self.inner.descriptor()
    }

    fn connect(
        &self,
        launch: LaunchSpec,
        supervisor: crate::Supervisor,
    ) -> BoxFuture<'_, Result<ProviderConnection, saturn_core::providers::ProviderError>> {
        self.inner.connect(launch, supervisor)
    }

    fn injectability(&self, _kind: ExtensionPartKind) -> Injectability {
        *self.answer.lock().unwrap()
    }
}

#[tokio::test]
async fn only_unknown_judgments_are_asked_again_at_start() {
    let mut flow = Flow::new(Vec::new()).await;
    let answer = Arc::new(Mutex::new(Injectability::Injectable));
    flow.engine
        .registry
        .register(Arc::new(Moody {
            inner: FakeAdapter {
                descriptor: fake_descriptor(NARROW),
                provider: FakeProvider::new(NARROW),
            },
            answer: Arc::clone(&answer),
        }))
        .unwrap();
    let mut client = flow.client().await;
    for (name, said) in [
        ("known-kit", Injectability::Injectable),
        ("unsure-kit", Injectability::Unknown),
    ] {
        let source = flow.fixture.root.path().join("sources").join(name);
        write(&source, "SKILL.md", "# a skill");
        *answer.lock().unwrap() = said;
        install(&mut flow, &source.display().to_string()).await;
    }
    *answer.lock().unwrap() = Injectability::Unavailable;

    flow.engine.rejudge_unknown_extensions().await;

    let listed = list(&mut flow, &mut client).await;
    assert_eq!(
        verdicts_of(&listed[0], NARROW),
        Some(Injectability::Injectable)
    );
    assert_eq!(
        verdicts_of(&listed[1], NARROW),
        Some(Injectability::Unavailable)
    );
}

fn verdicts_of(info: &ExtensionInfo, provider: Provider) -> Option<Injectability> {
    info.parts[0]
        .verdicts
        .iter()
        .find(|(id, _)| *id == provider)
        .map(|(_, verdict)| *verdict)
}

#[tokio::test]
async fn pruning_old_chats_leaves_installed_extensions_alone() {
    let mut flow = Flow::with_config("retention.max_age_days = 1\n", Vec::new()).await;
    let source = bundle(&flow, "review-kit");
    install(&mut flow, &source.display().to_string()).await;
    let old = flow
        .engine
        .store
        .create_chat(flow.fixture.workdir.clone())
        .await
        .unwrap();
    flow.engine.store.age_chat(old, 3).await;

    flow.engine.prune_records(CLIENT, true).await.unwrap();

    assert!(flow.engine.store.chat_workdir(old).await.is_err());
    assert_eq!(flow.engine.store.extension_rows().await.unwrap().len(), 1);
    assert!(store_dir(&flow).join("review-kit/.mcp.json").is_file());
}

/// 내려받기가 끝나기를 기다리는 가짜 git. 문 파일이 생기면 진짜 git을 실행한다.
fn gated_git(flow: &Flow, gate: &Path) -> PathBuf {
    let script = flow.fixture.root.path().join("fake-git");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nwhile [ ! -f '{}' ]; do sleep 0.02; done\nexec git \"$@\"\n",
            gate.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    script
}

#[tokio::test]
async fn a_slow_clone_does_not_hold_up_other_requests_and_is_finished_when_it_ends() {
    let mut flow = flow_with_two_adapters().await;
    let repo = flow.fixture.root.path().join("remotes/slow-kit.git");
    write(&repo, "SKILL.md", "# slow");
    for args in [
        vec!["init", "--quiet"],
        vec!["add", "."],
        vec![
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "commit",
            "--quiet",
            "-m",
            "x",
        ],
    ] {
        assert!(
            std::process::Command::new("git")
                .args(&args)
                .current_dir(&repo)
                .status()
                .unwrap()
                .success()
        );
    }
    let gate = flow.fixture.root.path().join("gate");
    flow.engine.flow.git_program = gated_git(&flow, &gate);
    let mut client = flow.client().await;
    let chat = flow.chat;
    let url = format!("file://{}", repo.display());

    // 내려받기가 막혀 있는 동안 다른 요청에 답이 온다
    let listed = drive(&mut flow.engine, async {
        client
            .send(5, Request::InstallExtension { chat, source: url })
            .await;
        assert_eq!(client.response().await.id, Some(RequestId(5)));
        let QueryResult::ExtensionList { extensions } =
            client.query(6, Request::ListExtensions).await
        else {
            panic!("expected the extension list");
        };
        extensions
    })
    .await;
    assert!(listed.is_empty());
    assert!(!store_dir(&flow).join("slow-kit/SKILL.md").exists());

    std::fs::write(&gate, "").unwrap();
    let after = drive(&mut flow.engine, async {
        loop {
            if let ServerMessage::Notification(message) = client.recv().await
                && let Notification::ChatNotice {
                    notice: notice @ ChatNotice::ExtensionInstalled { .. },
                    ..
                } = message.notification
            {
                return notice;
            }
        }
    })
    .await;
    assert_eq!(installed(&after).name, "slow-kit");
    assert_eq!(flow.engine.store.extension_rows().await.unwrap().len(), 1);
}

#[tokio::test]
async fn the_same_name_cannot_be_installed_again_while_it_is_being_downloaded() {
    let mut flow = flow_with_two_adapters().await;
    let gate = flow.fixture.root.path().join("gate");
    flow.engine.flow.git_program = gated_git(&flow, &gate);
    let mut client = flow.client().await;

    install(&mut flow, "file:///nowhere/busy-kit.git").await;
    install(&mut flow, "file:///elsewhere/busy-kit.git").await;

    let notices = extension_notices(&client.window().await);
    assert_eq!(notices.len(), 1);
    assert!(matches!(
        &notices[0],
        ChatNotice::ExtensionFailed { reason, .. } if reason.contains("already installed")
    ));
}
