//! provider 전환 안내 테스트: 전환할 때 새 provider가 받지 못하는 확장 부분만 전환 줄 바로 뒤에 알린다.
//! 설계: docs/design/extensions.md#옮길-수-없는-부분-알림

use std::sync::Arc;

use saturn_protocol::ids::Provider;
use saturn_protocol::rpc::{ChatNotice, ExtensionPartKind};

use super::support::{Flow, idle_reply, text, turn_completed};
use super::*;
use crate::providers::test_support::{CLAUDE, CODEX, FakeAdapter, FakeProvider, fake_descriptor};
use crate::providers::{ExtensionLayout, Registry};

/// 명령과 스킬을 받는 provider와 스킬만 받는 provider.
const WITH_COMMANDS: ExtensionLayout = ExtensionLayout {
    skills_dir: Some("skills"),
    commands: Some(("commands", "md")),
    mcp_servers: false,
    hooks: false,
};
const SKILLS_ONLY: ExtensionLayout = ExtensionLayout {
    skills_dir: Some("skills"),
    commands: None,
    mcp_servers: false,
    hooks: false,
};

fn registry(claude: ExtensionLayout, codex: ExtensionLayout) -> Registry {
    let mut registry = Registry::default();
    for (id, layout, order) in [(CLAUDE, claude, 10), (CODEX, codex, 20)] {
        let mut descriptor = fake_descriptor(id);
        descriptor.extensions = layout;
        descriptor.order = order;
        registry
            .register(Arc::new(FakeAdapter {
                descriptor,
                provider: FakeProvider::new(id),
            }))
            .unwrap();
    }
    registry
}

/// 스킬 하나와 명령 하나가 든 확장을 설치한 채팅. 첫 입력은 Claude가 끝낸 상태다.
async fn flow_after_first_turn(claude: ExtensionLayout, codex: ExtensionLayout) -> Flow {
    let replies = (0..3).map(|_| idle_reply(0.95)).collect();
    let mut flow = Flow::new(replies).await;
    flow.add_provider(CODEX);
    flow.engine.registry = registry(claude, codex);
    // 설치 요청은 이미 열린 연결을 다시 시작하므로, 기록에 부분 정의를 바로 넣어 연결은 그대로 둔다
    let parts = r#"[
        {"kind":"Skill","name":"commit-helper","path":"skills/commit-helper","verdicts":[]},
        {"kind":"Command","name":"review","path":"commands/review.md","verdicts":[]}
    ]"#;
    flow.engine
        .store
        .insert_extension("review-kit", "/src/review-kit", parts)
        .await
        .unwrap();
    flow.submit("write the cache module").await;
    let agent = flow.agent();
    flow.claude_event(text(agent, "done")).await;
    flow.claude_event(turn_completed(agent)).await;
    flow
}

/// 알림 순서대로 전환 줄과 확장 줄만.
async fn notices_after_switch(flow: &mut Flow, to: Provider) -> Vec<ChatNotice> {
    let mut client = flow.client().await;
    flow.engine.switch_provider(flow.chat, to);
    flow.submit("review the cache module").await;
    client
        .window()
        .await
        .into_iter()
        .filter_map(|notification| match notification {
            Notification::ChatNotice { notice, .. }
                if matches!(
                    notice,
                    ChatNotice::ProviderSwitched { .. }
                        | ChatNotice::ExtensionPartsNotApplied { .. }
                ) =>
            {
                Some(notice)
            }
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_switch_tells_only_the_parts_the_new_provider_loses() {
    let switched = ChatNotice::ProviderSwitched {
        from: CLAUDE,
        to: CODEX,
    };
    // (이름, 이전 확장 지원, 다음 확장 지원, 예상 알림)
    let cases = [
        (
            "tells which parts the new provider does not take right after the switch line",
            WITH_COMMANDS,
            SKILLS_ONLY,
            vec![
                switched.clone(),
                ChatNotice::ExtensionPartsNotApplied {
                    provider: CODEX,
                    parts: vec![(
                        "review-kit".to_owned(),
                        ExtensionPartKind::Command,
                        "review".to_owned(),
                    )],
                },
            ],
        ),
        (
            "a switch that loses nothing adds no line",
            WITH_COMMANDS,
            WITH_COMMANDS,
            vec![switched.clone()],
        ),
        (
            "parts neither provider takes are not told again",
            SKILLS_ONLY,
            SKILLS_ONLY,
            vec![switched],
        ),
    ];
    for (name, before, after, expected) in cases {
        let mut flow = flow_after_first_turn(before, after).await;

        let notices = notices_after_switch(&mut flow, CODEX).await;

        assert_eq!(notices, expected, "{name}");
    }
}
