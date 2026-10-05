//! 제약 인계 테스트: 저장한 유효 제약이 긴 대화 뒤 전환에서도 새 session의 패킷 제약 구역에 들어가고, 넣은 제약이 `packet_constraints`에 남는지 확인한다.
//! 설계: docs/design/constraints.md#패킷의-제약-칸

use saturn_protocol::ids::{ConstraintId, InputId, Provider, SessionId, TaskId};
use saturn_protocol::rpc::{ChatNotice, Notification};

use super::support::{Flow, idle_reply, text, tool_read, tool_result, turn_completed};
use crate::providers::test_support::{CLAUDE, CODEX, Call, FakeProvider};
use crate::store::{
    Actor, ConstraintChange, ConstraintState, NewChange, NewRegistration, NewRule, PacketKind,
    PacketState, sha256_hex,
};

/// 제약 원문. 어느 입력이나 답, 파일 내용에도 나오지 않아 최근 턴으로는 맞출 수 없다.
const RULE: &str = "Release builds must never print the internal build token";

const CODEX_FIRST: SessionId = SessionId(1);
const CLAUDE_FIRST: SessionId = SessionId(2);

async fn flow_with(config: &str) -> (Flow, FakeProvider) {
    let replies = (0..12).map(|_| idle_reply(0.95)).collect();
    let mut flow = Flow::with_config(config, replies).await;
    let codex = flow.add_provider(CODEX);
    (flow, codex)
}

async fn register(flow: &Flow, input: InputId, rules: &[(&str, &[&str])]) -> Vec<ConstraintId> {
    let rules: Vec<NewRule> = rules
        .iter()
        .enumerate()
        .map(|(index, (rule, scope))| NewRule {
            line: u32::try_from(index).unwrap(),
            rule: (*rule).to_owned(),
            scope: scope.iter().map(|path| (*path).to_owned()).collect(),
        })
        .collect();
    flow.engine
        .store
        .register_constraints(&NewRegistration {
            chat: flow.chat,
            input,
            rules: &rules,
            state: ConstraintState::Active,
            actor: Actor::User,
            reason: None,
            judgment: None,
        })
        .await
        .unwrap()
        .constraints
}

pub(super) async fn turn(flow: &mut Flow, provider: Provider, input: &str, call: &str) -> InputId {
    let id = flow.submit(input).await;
    let agent = flow.agent();
    flow.event(provider, text(agent, &format!("done {call}")))
        .await;
    flow.event(provider, tool_read(agent, call, "src/cache.rs"))
        .await;
    flow.event(provider, tool_result(agent, call, "plain file body"))
        .await;
    flow.event(provider, turn_completed(agent)).await;
    id
}

pub(super) fn packet_of(fake: &FakeProvider) -> String {
    fake.calls()
        .into_iter()
        .find_map(|call| match call {
            Call::Open { packet, .. } => packet,
            _ => None,
        })
        .expect("the new session should get a packet")
}

/// 긴 대화를 Codex로 한 뒤 Claude로 바꿔 Claude가 받은 첫 입력(패킷)과 그 session을 돌려준다.
async fn long_chat_then_switch(flow: &mut Flow, rules: &[(&str, &[&str])]) -> String {
    let claude = flow.fake.clone();
    flow.engine.switch_provider(flow.chat, CODEX);
    let first = turn(flow, CODEX, "task 1 write the cache", "c1").await;
    register(flow, first, rules).await;
    for number in 2..=6 {
        turn(
            flow,
            CODEX,
            &format!("task {number} continue the cache"),
            &format!("c{number}"),
        )
        .await;
    }
    flow.engine.switch_provider(flow.chat, CLAUDE);
    flow.submit("task 7 review the cache").await;
    packet_of(&claude)
}

type PacketRows = Vec<(ConstraintId, String)>;

/// 새 session의 기록에 칸이 차서 빠진 제약이 있으면 대화 기록에 그 수를 알렸는지 본다. 빠진 제약이 없으면 알리지 않는 것이므로 보지 않는다.
async fn assert_omitted_notice(client: &mut super::Client, rows: &PacketRows) {
    let count = rows.iter().filter(|(_, tier)| tier == "Omitted").count();
    let Some(count) = u32::try_from(count).ok().filter(|count| *count > 0) else {
        return;
    };
    let notified = client
        .until(|notification| match notification {
            Notification::ChatNotice {
                notice: ChatNotice::ConstraintsOmitted { count },
                ..
            } => Some(*count),
            _ => None,
        })
        .await;
    assert_eq!(notified, count);
}

#[tokio::test]
async fn long_chat_constraints_reach_the_new_sessions_packet_and_record() {
    struct Case {
        name: &'static str,
        config: &'static str,
        rules: Vec<(String, Vec<&'static str>)>,
        check: fn(&str, &str, &PacketRows, &PacketRows), // 패킷, Claude session 기록, Codex session 기록
    }
    let cases = [
        Case {
            name: "a constraint stays in the packet when it left the recent turns",
            config: "",
            rules: vec![(RULE.to_owned(), Vec::new())],
            check: |name, packet, _, _| {
                let (fixed, rest) = packet
                    .split_once("## Goal and last input")
                    .expect("goal section should exist");
                assert!(
                    fixed.contains("## Constraints and decisions"),
                    "{name}: {packet}"
                );
                assert!(fixed.contains(RULE), "{name}: {packet}");
                assert!(
                    !rest.contains(RULE),
                    "{name}: rule leaked into other zones: {packet}"
                );
                assert!(
                    !packet.contains("task 1 write the cache\nAgent"),
                    "{name}: {packet}"
                );
            },
        },
        Case {
            name: "packet constraints record what the new session got",
            config: "",
            rules: vec![
                (RULE.to_owned(), Vec::new()),
                ("Keep api.rs stable".to_owned(), vec!["src/api.rs"]),
            ],
            check: |name, packet, claude_rows, codex_rows| {
                assert_eq!(
                    *claude_rows,
                    vec![
                        (ConstraintId(1), "All".to_owned()),
                        (ConstraintId(2), "Relevance".to_owned()),
                    ],
                    "{name}"
                );
                assert!(codex_rows.is_empty(), "{name}");
                assert!(packet.contains(RULE), "{name}: {packet}");
                assert!(packet.contains("Keep api.rs stable"), "{name}: {packet}");
                assert!(!packet.contains("Constraints omitted"), "{name}: {packet}");
            },
        },
        Case {
            // 새 session의 provider 기준 P_max는 20_000토큰이고 그 1%라 제약 칸은 200토큰(800자)이다
            name: "constraints over the slot are omitted and marked",
            config: "[context]\nconstraint_slot_percent = 1\n",
            rules: (1..=4)
                .map(|number| (format!("rule{number} {}", "x".repeat(300)), Vec::new()))
                .collect(),
            check: |name, packet, claude_rows, _| {
                // 한 줄 306글자 + 구분 2글자라 800자 칸에는 최신 둘만 들어간다
                assert!(packet.contains("rule4 "), "{name}: {packet}");
                assert!(packet.contains("rule3 "), "{name}: {packet}");
                assert!(!packet.contains("rule2 "), "{name}: {packet}");
                assert!(!packet.contains("rule1 "), "{name}: {packet}");
                assert!(
                    packet.contains("Constraints omitted: 2"),
                    "{name}: {packet}"
                );
                let tiers: Vec<&str> = claude_rows.iter().map(|(_, tier)| tier.as_str()).collect();
                assert_eq!(tiers, ["Omitted", "Omitted", "All", "All"], "{name}");
            },
        },
    ];

    for case in cases {
        let (mut flow, _codex) = flow_with(case.config).await;
        let rules: Vec<(&str, &[&str])> = case
            .rules
            .iter()
            .map(|(rule, scope)| (rule.as_str(), scope.as_slice()))
            .collect();
        let mut client = flow.client().await;
        let packet = long_chat_then_switch(&mut flow, &rules).await;
        let claude_rows = flow
            .engine
            .store
            .packet_constraints_of(CLAUDE_FIRST)
            .await
            .unwrap();
        let codex_rows = flow
            .engine
            .store
            .packet_constraints_of(CODEX_FIRST)
            .await
            .unwrap();
        assert_omitted_notice(&mut client, &claude_rows).await;
        (case.check)(case.name, &packet, &claude_rows, &codex_rows);
        assert_packet_record(&flow, &packet, &claude_rows).await;
    }
}

/// Claude가 받은 패킷의 전달 기록이 받은 글의 해시와 일치하고, 제약 칸 항목이 `packet_constraints`와 같다.
async fn assert_packet_record(flow: &Flow, packet: &str, claude_rows: &PacketRows) {
    let recorded = flow.engine.store.packets_of_chat(flow.chat).await.unwrap();
    let stored = recorded
        .iter()
        .find(|stored| stored.session == CLAUDE_FIRST)
        .expect("the switch packet should be recorded");
    assert_eq!(stored.body_hash, sha256_hex(packet.as_bytes()));
    assert_eq!(stored.body_bytes, packet.len() as u64);
    assert_eq!((stored.kind, stored.attempt), (PacketKind::Switch, 1));
    assert_eq!(stored.state, PacketState::Sent);
    assert!(stored.provider_session.is_some() && stored.run.is_some());
    let items = flow.engine.store.packet_items(stored.id).await.unwrap();
    let in_slot: Vec<(u64, &str, Option<&str>)> = items
        .iter()
        .filter(|item| item.0 == "Constraints")
        .map(|item| (item.1, item.2.as_deref().unwrap_or("-"), item.3.as_deref()))
        .collect();
    let expected: Vec<(u64, &str, Option<&str>)> = claude_rows
        .iter()
        .map(|(id, tier)| match tier.as_str() {
            "Omitted" => (id.0, "-", Some("slot_full")),
            tier => (id.0, tier, None),
        })
        .collect();
    assert_eq!(in_slot, expected);
}

#[tokio::test]
async fn released_constraint_is_not_handed_over() {
    let (mut flow, _codex) = flow_with("").await;
    flow.engine.switch_provider(flow.chat, CODEX);
    let first = turn(&mut flow, CODEX, "task 1 write the cache", "c1").await;
    register(&flow, first, &[(RULE, &[])]).await;
    flow.engine
        .store
        .release_input_constraints(first)
        .await
        .unwrap();
    let claude = flow.fake.clone();
    flow.engine.switch_provider(flow.chat, CLAUDE);

    flow.submit("task 2 review the cache").await;

    let packet = packet_of(&claude);
    assert!(!packet.contains(RULE), "{packet}");
    assert!(
        flow.engine
            .store
            .packet_constraints_of(CLAUDE_FIRST)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn constraint_with_an_exception_is_handed_over_with_its_exception_note() {
    let (mut flow, _codex) = flow_with("").await;
    flow.engine.switch_provider(flow.chat, CODEX);
    let first = turn(&mut flow, CODEX, "task 1 write the cache", "c1").await;
    let paused = "Keep every cache key lowercase";
    let ids = register(&flow, first, &[(RULE, &[]), (paused, &[])]).await;
    for (id, change) in [
        (ids[0], ConstraintChange::Once { task: TaskId(999) }),
        (
            ids[1],
            ConstraintChange::Scoped {
                condition: "only inside tests/".to_owned(),
            },
        ),
    ] {
        let revision = flow
            .engine
            .store
            .constraint_revision(flow.chat)
            .await
            .unwrap();
        flow.engine
            .store
            .change_constraint(&NewChange {
                chat: flow.chat,
                constraint: id,
                change,
                actor: Actor::Router,
                reason: None,
                input: None,
                judgment: None,
                revision,
            })
            .await
            .unwrap();
    }
    let claude = flow.fake.clone();
    flow.engine.switch_provider(flow.chat, CLAUDE);

    flow.submit("task 2 review the cache").await;

    let packet = packet_of(&claude);
    assert!(
        packet.contains(&format!("{RULE} [paused for the current task]")),
        "{packet}"
    );
    assert!(
        packet.contains(&format!("{paused} [exception: only inside tests/]")),
        "{packet}"
    );
}
