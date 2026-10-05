//! 확장 주입 테스트: 어댑터는 자기가 주입 가능하다고 답한 부분만 받고, 설치와 제거는 영향을 받는 연결만 다시 시작하게 한다.
//! 설계: docs/design/extensions.md#주입

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use saturn_protocol::ids::Provider;
use saturn_protocol::rpc::{ChatNotice, ExtensionPartKind, Notification};

use super::support::{CLIENT, Flow};
use crate::providers::test_support::{FakeAdapter, FakeProvider, fake_descriptor};
use crate::providers::{
    Adapter, BoxFuture, ExtensionLayout, InjectionFailure, LaunchSpec, PermissionInput,
    PermissionLaunch, ProviderConnection,
};

const NARROW: Provider = Provider::from_static("narrow-agent");
const WIDE: Provider = Provider::from_static("wide-agent");
const NONE: Provider = Provider::from_static("plain-agent");

const NARROW_LAYOUT: ExtensionLayout = ExtensionLayout {
    skills_dir: Some("skills"),
    commands: None,
    mcp_servers: true,
    hooks: false,
};
const WIDE_LAYOUT: ExtensionLayout = ExtensionLayout {
    skills_dir: Some("skills"),
    commands: Some(("commands", "md")),
    mcp_servers: true,
    hooks: true,
};

/// 어댑터가 받은 부분: 확장 이름, 종류, 부분 이름, 원본 경로.
type Seen = (String, ExtensionPartKind, String, PathBuf);

/// 어댑터가 부른 때마다 받은 부분과 지문.
type Log = Arc<Mutex<Vec<(Vec<Seen>, String)>>>;

/// 받은 확장 입력을 기록하고, 정해 둔 주입 실패를 돌려주는 어댑터.
#[derive(Debug)]
struct Recording {
    inner: FakeAdapter,
    seen: Log,
    failures: Vec<InjectionFailure>,
}

impl Adapter for Recording {
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

    fn translate_permission(
        &self,
        input: PermissionInput<'_>,
    ) -> Result<PermissionLaunch, saturn_core::providers::ProviderError> {
        let parts = input
            .extensions
            .parts
            .iter()
            .map(|part| {
                (
                    part.extension.clone(),
                    part.kind,
                    part.name.clone(),
                    part.source.clone(),
                )
            })
            .collect();
        self.seen
            .lock()
            .unwrap()
            .push((parts, input.extensions.fingerprint.to_owned()));
        Ok(PermissionLaunch {
            questions_disabled: !input.questions,
            injection_failures: self.failures.clone(),
            ..PermissionLaunch::default()
        })
    }
}

fn register(
    flow: &mut Flow,
    id: Provider,
    layout: ExtensionLayout,
    failures: Vec<InjectionFailure>,
) -> Log {
    let mut descriptor = fake_descriptor(id);
    descriptor.extensions = layout;
    let seen = Log::default();
    flow.engine
        .registry
        .register(Arc::new(Recording {
            inner: FakeAdapter {
                descriptor,
                provider: FakeProvider::new(id),
            },
            seen: Arc::clone(&seen),
            failures,
        }))
        .unwrap();
    seen
}

fn write(root: &std::path::Path, file: &str, content: &str) {
    let path = root.join(file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

/// 스킬, 명령, MCP 서버, 훅이 하나씩 든 묶음을 설치한다.
async fn install_bundle(flow: &mut Flow, name: &str) {
    let root = flow.fixture.root.path().join("sources").join(name);
    write(&root, "skills/commit-helper/SKILL.md", "# commit helper");
    write(&root, "commands/review.md", "review the diff");
    write(
        &root,
        ".mcp.json",
        r#"{"mcpServers":{"lint":{"command":"lint"}}}"#,
    );
    write(&root, "hooks/hooks.json", r#"{"hooks":{"PreToolUse":[]}}"#);
    flow.engine
        .install_extension(CLIENT, flow.chat, &root.display().to_string())
        .await
        .unwrap();
}

/// 실제 시작과 같은 순서로 연결을 만들어 시작 때의 확장 지문을 기억하게 한다.
async fn connect(flow: &mut Flow, provider: Provider) {
    let revision = flow.engine.settings.current().unwrap();
    let launch = flow
        .engine
        .launch_spec(provider, flow.chat, revision)
        .await
        .unwrap();
    let seed = crate::launch::ConnectionSeed::of(&launch);
    let connection = flow
        .engine
        .registry
        .connect(launch, flow.engine.supervisor.clone())
        .await
        .unwrap();
    flow.engine.attach_connection(flow.chat, connection, seed);
}

async fn launch(flow: &Flow, provider: Provider) -> PermissionLaunch {
    let revision = flow.engine.settings.current().unwrap();
    flow.engine
        .launch_spec(provider, flow.chat, revision)
        .await
        .unwrap()
        .permission
}

fn last(log: &Log) -> (Vec<Seen>, String) {
    log.lock()
        .unwrap()
        .last()
        .cloned()
        .expect("adapter should have been asked")
}

fn kinds(parts: &[Seen]) -> Vec<(ExtensionPartKind, &str)> {
    parts
        .iter()
        .map(|(_, kind, name, _)| (*kind, name.as_str()))
        .collect()
}

fn inject_notices(notifications: &[Notification]) -> Vec<ChatNotice> {
    notifications
        .iter()
        .filter_map(|notification| match notification {
            Notification::ChatNotice { notice, .. }
                if matches!(notice, ChatNotice::ExtensionInjectFailed { .. }) =>
            {
                Some(notice.clone())
            }
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn an_adapter_receives_only_the_parts_it_can_inject() {
    let mut flow = Flow::new(Vec::new()).await;
    let narrow = register(&mut flow, NARROW, NARROW_LAYOUT, Vec::new());
    let wide = register(&mut flow, WIDE, WIDE_LAYOUT, Vec::new());
    let plain = register(&mut flow, NONE, ExtensionLayout::NONE, Vec::new());
    install_bundle(&mut flow, "review-kit").await;

    let narrow_launch = launch(&flow, NARROW).await;
    launch(&flow, WIDE).await;
    launch(&flow, NONE).await;

    let (parts, fingerprint) = last(&narrow);
    assert_eq!(
        kinds(&parts),
        vec![
            (ExtensionPartKind::Skill, "commit-helper"),
            (ExtensionPartKind::McpServer, "lint")
        ]
    );
    assert!(!fingerprint.is_empty());
    assert_eq!(narrow_launch.extension_fingerprint, fingerprint);
    let store = flow.fixture.options.home.join("extensions/review-kit");
    assert_eq!(parts[0].3, store.join("skills/commit-helper"));
    assert_eq!(parts[1].3, store.join(".mcp.json"));
    let (wide_parts, wide_fingerprint) = last(&wide);
    assert_eq!(wide_parts.len(), 4);
    assert_ne!(wide_fingerprint, fingerprint);
    assert_eq!(last(&plain), (Vec::new(), String::new()));
}

#[tokio::test]
async fn without_installed_extensions_the_adapter_gets_nothing() {
    let mut flow = Flow::new(Vec::new()).await;
    let wide = register(&mut flow, WIDE, WIDE_LAYOUT, Vec::new());

    let permission = launch(&flow, WIDE).await;

    assert_eq!(last(&wide), (Vec::new(), String::new()));
    assert_eq!(permission.extension_fingerprint, "");
}

#[tokio::test]
async fn a_missing_original_is_told_and_the_other_extensions_are_still_injected() {
    let mut flow = Flow::new(Vec::new()).await;
    let narrow = register(&mut flow, NARROW, NARROW_LAYOUT, Vec::new());
    install_bundle(&mut flow, "gone-kit").await;
    install_bundle(&mut flow, "kept-kit").await;
    std::fs::remove_dir_all(flow.fixture.options.home.join("extensions/gone-kit")).unwrap();
    let mut client = flow.client().await;

    launch(&flow, NARROW).await;

    let (parts, _) = last(&narrow);
    assert!(parts.iter().all(|(extension, ..)| extension == "kept-kit"));
    assert_eq!(parts.len(), 2);
    assert_eq!(
        inject_notices(&client.window().await),
        vec![ChatNotice::ExtensionInjectFailed {
            extension: "gone-kit".to_owned(),
            part: None,
            provider: NARROW,
            reason: "the original is missing from the extension store".to_owned(),
        }]
    );
    assert_eq!(flow.engine.store.extension_rows().await.unwrap().len(), 2);
}

#[tokio::test]
async fn a_part_the_adapter_could_not_inject_is_told_with_its_provider() {
    let mut flow = Flow::new(Vec::new()).await;
    register(
        &mut flow,
        NARROW,
        NARROW_LAYOUT,
        vec![InjectionFailure {
            extension: "review-kit".to_owned(),
            part: "lint".to_owned(),
            reason: "bad definition".to_owned(),
        }],
    );
    install_bundle(&mut flow, "review-kit").await;
    let mut client = flow.client().await;

    launch(&flow, NARROW).await;

    assert_eq!(
        inject_notices(&client.window().await),
        vec![ChatNotice::ExtensionInjectFailed {
            extension: "review-kit".to_owned(),
            part: Some("lint".to_owned()),
            provider: NARROW,
            reason: "bad definition".to_owned(),
        }]
    );
}

fn restarted(notifications: &[Notification]) -> Vec<Provider> {
    notifications
        .iter()
        .filter_map(|notification| match notification {
            Notification::ChatNotice {
                notice: ChatNotice::ProviderRestarted { provider },
                ..
            } => Some(*provider),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn an_install_restarts_only_the_connections_whose_parts_changed() {
    let mut flow = Flow::new(Vec::new()).await;
    register(&mut flow, NARROW, NARROW_LAYOUT, Vec::new());
    register(&mut flow, NONE, ExtensionLayout::NONE, Vec::new());
    for provider in [NARROW, NONE] {
        connect(&mut flow, provider).await;
    }
    let mut client = flow.client().await;

    install_bundle(&mut flow, "review-kit").await;

    // 시험 흐름이 이미 열어 둔 Claude 연결도 같은 이유로 다시 시작하므로 두 가짜만 본다
    let told = restarted(&client.window().await);
    assert!(told.contains(&NARROW) && !told.contains(&NONE));
    assert!(!flow.engine.providers.contains_key(&(flow.chat, NARROW)));
    assert!(flow.engine.providers.contains_key(&(flow.chat, NONE)));
}

#[tokio::test]
async fn a_remove_restarts_the_connection_that_had_the_parts() {
    let mut flow = Flow::new(Vec::new()).await;
    register(&mut flow, NARROW, NARROW_LAYOUT, Vec::new());
    install_bundle(&mut flow, "review-kit").await;
    connect(&mut flow, NARROW).await;
    let mut client = flow.client().await;

    flow.engine
        .remove_extension(CLIENT, flow.chat, "review-kit")
        .await
        .unwrap();

    assert!(restarted(&client.window().await).contains(&NARROW));
}

#[tokio::test]
async fn an_extension_with_nothing_for_a_provider_leaves_its_connection_alone() {
    let mut flow = Flow::new(Vec::new()).await;
    register(&mut flow, NARROW, NARROW_LAYOUT, Vec::new());
    connect(&mut flow, NARROW).await;
    let hook_only = flow.fixture.root.path().join("sources/hook-kit");
    write(
        &hook_only,
        "hooks/hooks.json",
        r#"{"hooks":{"PreToolUse":[]}}"#,
    );
    let mut client = flow.client().await;

    flow.engine
        .install_extension(CLIENT, flow.chat, &hook_only.display().to_string())
        .await
        .unwrap();

    assert!(!restarted(&client.window().await).contains(&NARROW));
    assert!(flow.engine.providers.contains_key(&(flow.chat, NARROW)));
}
