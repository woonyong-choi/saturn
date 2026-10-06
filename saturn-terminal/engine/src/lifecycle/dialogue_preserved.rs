//! 대화 보존 테스트: 기록된 사용자 입력과 에이전트 글이 전환 패킷에 줄거나 빠지지 않고 실제 순서로 실리고, 전달 패킷 기록의 번호·해시가 보낸 글과 맞는지 확인한다.
//! 설계: docs/design/context-management.md#보존-우선-경로

use saturn_core::providers::ProviderError;
use saturn_protocol::ids::SessionId;
use saturn_protocol::state::{InputState, SessionState};

use super::constraint_handoff::{packet_of, turn};
use super::support::{Flow, idle_reply, text, tool_read, tool_result, turn_completed};
use crate::providers::test_support::{CLAUDE, CODEX};
use crate::store::{PacketState, StoredPacket, sha256_hex};

const CODEX_FIRST: SessionId = SessionId(1);
const CLAUDE_FIRST: SessionId = SessionId(2);

/// Claude의 `P_max`를 400토큰(1,600자)으로 줄인다. 지시문도 이 안에 든다. 창은 그대로라 전송 가능 상한은 넉넉하다.
const TIGHT: &str = "[provider.claude.context]\nt_abs = 4000\n";

/// `TIGHT`에 더해 Claude의 창을 예약량 62,000토큰에 1,000토큰만 더한 크기로 줄여 전송 가능 상한 `P_send`를 700토큰(안전 비율 70%)으로 만든다.
const SEND_TIGHT: &str = "[provider.claude.context]\nt_abs = 4000\nwindow = 63000\n";

const CORRECTION: &str = "actually the header is X-Route-Key, not X-Req-Id";

async fn flow_with(config: &str) -> Flow {
    let replies = (0..12).map(|_| idle_reply(0.95)).collect();
    let mut flow = Flow::with_config(config, replies).await;
    flow.add_provider(CODEX);
    flow.engine.switch_provider(flow.chat, CODEX);
    flow
}

/// Codex가 입력 하나에 `answer`로 답하고 `call`을 읽은 결과가 `body`인 턴을 끝낸다.
async fn long_turn(flow: &mut Flow, input: &str, answer: &str, (call, body): (&str, &str)) {
    flow.submit(input).await;
    let agent = flow.agent();
    flow.event(CODEX, text(agent, answer)).await;
    flow.event(CODEX, tool_read(agent, call, "src/cache.rs"))
        .await;
    flow.event(CODEX, tool_result(agent, call, body)).await;
    flow.event(CODEX, turn_completed(agent)).await;
}

async fn stored_of(flow: &Flow, session: SessionId) -> Vec<StoredPacket> {
    flow.engine
        .store
        .packets_of_chat(flow.chat)
        .await
        .unwrap()
        .into_iter()
        .filter(|stored| stored.session == session)
        .collect()
}

/// 보낸 패킷 시도의 대화 본문 행을 `(구역, 원문 해시)`로.
async fn dialogue_of(flow: &Flow, stored: &StoredPacket) -> Vec<(String, String)> {
    flow.engine
        .store
        .packet_dialogue(stored.id)
        .await
        .unwrap()
        .into_iter()
        .map(|(zone, _, hash)| (zone, hash))
        .collect()
}

/// 입력과 답이 한 쌍씩인 대화의 기록 행. 입력은 `User`, 답은 `Assistant`다.
fn expected_rows(pairs: &[(&str, String)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .flat_map(|(input, answer)| {
            [
                ("User".to_owned(), sha256_hex(input.as_bytes())),
                ("Assistant".to_owned(), sha256_hex(answer.as_bytes())),
            ]
        })
        .collect()
}

fn assert_in_order(packet: &str, needles: &[String]) {
    let mut at = 0;
    for needle in needles {
        let found = packet[at..]
            .find(needle.as_str())
            .unwrap_or_else(|| panic!("{needle:?} should follow the previous item: {packet}"));
        at += found + needle.len();
    }
}

// #592: 정정 뒤에 읽기 턴이 네 개 이어져도 정정은 판단 기록과 제약 등록 없이 패킷의 제자리에 남고, 기록의 번호·해시와 순서가 맞는다
#[tokio::test]
async fn four_reads_after_a_correction_do_not_push_it_out_of_the_switch_packet() {
    four_reads_after_a_correction(true).await;
}

// #592: `/record off`라 판단 기록이 없어도 같다
#[tokio::test]
async fn four_reads_after_a_correction_survive_with_recording_off() {
    four_reads_after_a_correction(false).await;
}

async fn four_reads_after_a_correction(is_recording: bool) {
    let mut flow = flow_with("").await;
    flow.engine
        .store
        .set_recording(flow.chat, is_recording)
        .await
        .unwrap();
    let claude = flow.fake.clone();
    let inputs = [
        "use the header X-Req-Id",
        CORRECTION,
        "read the docs",
        "read the layout",
        "read the tests",
        "read the build",
    ];
    let mut pairs: Vec<(&str, String)> = Vec::new();
    for (number, input) in inputs.into_iter().enumerate() {
        turn(&mut flow, CODEX, input, &format!("c{number}")).await;
        pairs.push((input, format!("done c{number}")));
    }
    flow.engine.switch_provider(flow.chat, CLAUDE);

    let sent = flow.submit("now apply the header").await;

    assert_eq!(flow.state(sent), InputState::Applied);
    let packet = packet_of(&claude);
    assert_eq!(packet.matches(CORRECTION).count(), 1, "{packet}");
    let needles: Vec<String> = pairs
        .iter()
        .flat_map(|(input, answer)| [format!("User: {input}\nAgent: {answer}")])
        .collect();
    assert_in_order(&packet, &needles);
    let stored = stored_of(&flow, CLAUDE_FIRST).await;
    let [stored] = &stored[..] else {
        panic!("one packet attempt should be recorded: {stored:?}");
    };
    assert_eq!(stored.state, PacketState::Sent);
    assert_eq!(stored.body_hash, sha256_hex(packet.as_bytes()));
    assert_eq!(dialogue_of(&flow, stored).await, expected_rows(&pairs));
}

// #592: 보호 본문이 목표 예산을 넘어도 도구 구역만 비우고 본문은 한 글자도 줄이지 않고 보낸다
#[tokio::test]
async fn dialogue_over_the_target_budget_is_sent_whole_and_only_the_tools_are_dropped() {
    let mut flow = flow_with(TIGHT).await;
    let claude = flow.fake.clone();
    let answers: Vec<String> = ["a", "b", "c"]
        .iter()
        .map(|mark| format!("{mark}{}", ".".repeat(599)))
        .collect();
    for (number, answer) in answers.iter().enumerate() {
        long_turn(
            &mut flow,
            &format!("step {number}"),
            answer,
            (&format!("c{number}"), "TOOLBODY"),
        )
        .await;
    }
    flow.engine.switch_provider(flow.chat, CLAUDE);

    let sent = flow.submit("continue").await;

    assert_eq!(flow.state(sent), InputState::Applied);
    let packet = packet_of(&claude);
    for answer in &answers {
        assert!(packet.contains(answer.as_str()), "{packet}");
    }
    assert!(!packet.contains("TOOLBODY"), "{packet}");
    let stored = stored_of(&flow, CLAUDE_FIRST).await;
    let [stored] = &stored[..] else {
        panic!("one packet attempt should be recorded: {stored:?}");
    };
    let items = flow.engine.store.packet_items(stored.id).await.unwrap();
    let competing: Vec<Option<&str>> = items
        .iter()
        .filter(|item| item.0 == "Competing")
        .map(|item| item.3.as_deref())
        .collect();
    assert!(!competing.is_empty());
    assert!(competing.iter().all(|reason| *reason == Some("budget")));
}

// #592: 보호 본문이 전송 상한도 넘으면 어떤 본문도 자르지 않고 보내지 않으며, session과 입력은 그대로다
#[tokio::test]
async fn dialogue_over_the_send_limit_is_not_applied_and_nothing_is_cut() {
    let mut flow = flow_with(SEND_TIGHT).await;
    let claude = flow.fake.clone();
    for number in 0..6 {
        long_turn(
            &mut flow,
            &format!("step {number}"),
            &"a".repeat(600),
            (&format!("c{number}"), "TOOLBODY"),
        )
        .await;
    }
    flow.engine.switch_provider(flow.chat, CLAUDE);

    let held = flow.submit("continue").await;

    assert_eq!(flow.state(held), InputState::Held);
    assert!(claude.calls().is_empty());
    assert!(stored_of(&flow, CLAUDE_FIRST).await.is_empty());
    let old = flow.engine.sessions.get(CODEX_FIRST).unwrap();
    assert_eq!(old.state, SessionState::Open);
}

// #592: 맥락 한도로 거절돼 줄여 다시 보내도 대화 본문은 두 시도 모두 같은 번호·해시·순서로 남고, 줄어드는 것은 도구 구역뿐이다
#[tokio::test]
async fn reduced_resend_keeps_the_same_dialogue_and_only_drops_tools() {
    let mut flow = flow_with("").await;
    let claude = flow.fake.clone();
    claude.answer_open([
        Err(ProviderError::ContextExceeded { limit_tokens: None }),
        Ok(()),
    ]);
    let inputs = ["first request", CORRECTION, "read the docs"];
    let mut pairs: Vec<(&str, String)> = Vec::new();
    for (number, input) in inputs.into_iter().enumerate() {
        let body = format!("{number} body {}", "y".repeat(400));
        long_turn(
            &mut flow,
            input,
            &format!("done c{number}"),
            (&format!("c{number}"), &body),
        )
        .await;
        pairs.push((input, format!("done c{number}")));
    }
    flow.engine.switch_provider(flow.chat, CLAUDE);

    let sent = flow.submit("continue").await;

    assert_eq!(flow.state(sent), InputState::Applied);
    let packets: Vec<String> = claude
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            crate::providers::test_support::Call::Open { packet, .. } => packet,
            _ => None,
        })
        .collect();
    let [first, again] = &packets[..] else {
        panic!("the packet should be sent twice: {}", packets.len());
    };
    assert!(again.len() < first.len());
    let needles: Vec<String> = pairs
        .iter()
        .map(|(input, answer)| format!("User: {input}\nAgent: {answer}"))
        .collect();
    assert_in_order(first, &needles);
    assert_in_order(again, &needles);
    let attempts = stored_of(&flow, CLAUDE_FIRST)
        .await
        .into_iter()
        .chain(stored_of(&flow, SessionId(3)).await)
        .collect::<Vec<_>>();
    let [one, two] = &attempts[..] else {
        panic!("both attempts should be recorded: {attempts:?}");
    };
    assert_eq!(
        (one.state, two.state),
        (PacketState::NotSent, PacketState::Sent)
    );
    assert_eq!(two.reduced_from, Some(one.id));
    let expected = expected_rows(&pairs);
    assert_eq!(dialogue_of(&flow, one).await, expected);
    assert_eq!(dialogue_of(&flow, two).await, expected);
}

// #592: 턴 사이에 권한을 읽기 전용으로 줄여도 패킷은 그 뒤 입력 기준으로 만들어져, 대화 본문이 그대로 한 번만 실리고 기록의 대화 행도 같다
#[tokio::test]
async fn permission_reduced_before_the_switch_still_sends_the_whole_dialogue_once() {
    let mut flow = flow_with("").await;
    let claude = flow.fake.clone();
    let inputs = ["use the header X-Req-Id", CORRECTION, "read the docs"];
    let mut pairs: Vec<(&str, String)> = Vec::new();
    for (number, input) in inputs.into_iter().enumerate() {
        turn(&mut flow, CODEX, input, &format!("c{number}")).await;
        pairs.push((input, format!("done c{number}")));
    }
    flow.engine
        .set_permission_mode(flow.chat, "read-only")
        .await
        .unwrap();
    flow.engine.switch_provider(flow.chat, CLAUDE);

    let sent = flow.submit("now apply the header").await;

    assert_eq!(flow.state(sent), InputState::Applied);
    assert_eq!(
        flow.record(sent).permission,
        saturn_core::queue::Permission::ReadOnly
    );
    let packet = packet_of(&claude);
    assert_eq!(packet.matches(CORRECTION).count(), 1, "{packet}");
    let needles: Vec<String> = pairs
        .iter()
        .map(|(input, answer)| format!("User: {input}\nAgent: {answer}"))
        .collect();
    assert_in_order(&packet, &needles);
    let stored = stored_of(&flow, CLAUDE_FIRST).await;
    let [stored] = &stored[..] else {
        panic!("one packet attempt should be recorded: {stored:?}");
    };
    assert_eq!(stored.input, Some(sent));
    assert_eq!(stored.body_hash, sha256_hex(packet.as_bytes()));
    assert_eq!(dialogue_of(&flow, stored).await, expected_rows(&pairs));
}

/// 캡처 폴더의 파일 하나를 JSON으로 읽는다. 파일이 없으면 빈 목록.
fn captures_in(dir: &std::path::Path) -> Vec<serde_json::Value> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<_> = entries.map(|entry| entry.unwrap().path()).collect();
    files.sort();
    files
        .iter()
        .map(|path| serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap())
        .collect()
}

// #540: 켠 실험에서 캡처한 본문의 해시가 provider에 넘긴 인자와 같고 기록 행과 맞는다. 켜지 않으면 파일이 없다
#[tokio::test]
async fn packet_capture_matches_the_send_argument_and_is_absent_when_disabled() {
    for is_enabled in [true, false] {
        let mut flow = flow_with("").await;
        let dir = tempfile::tempdir().unwrap();
        let capture = dir.path().join("packet-capture");
        if is_enabled {
            flow.engine.packet_capture = Some(capture.clone());
        }
        let claude = flow.fake.clone();
        turn(&mut flow, CODEX, "use the header X-Req-Id", "c0").await;
        flow.engine.switch_provider(flow.chat, CLAUDE);
        flow.submit("now apply the header").await;

        let sent = packet_of(&claude);
        let stored = stored_of(&flow, CLAUDE_FIRST).await;
        let [stored] = &stored[..] else {
            panic!("one packet attempt should be recorded: {stored:?}");
        };
        let captured = captures_in(&capture);
        if !is_enabled {
            assert!(captured.is_empty(), "{captured:?}");
            assert!(!capture.exists());
            continue;
        }
        let [captured] = &captured[..] else {
            panic!("one capture per attempt: {captured:?}");
        };
        assert_eq!(captured["body"], sent);
        assert_eq!(captured["body_hash"], sha256_hex(sent.as_bytes()));
        assert_eq!(captured["body_hash"], stored.body_hash);
        assert_eq!(captured["packet_id"], stored.id.0);
        assert_eq!(captured["session"], CLAUDE_FIRST.0);
        assert_eq!(captured["provider"], "claude");
        assert_eq!(captured["attempt"], 1);
        assert!(
            captured["captured_at_unix_us"]
                .as_str()
                .unwrap()
                .parse::<u128>()
                .unwrap()
                > 0
        );
        use std::os::unix::fs::PermissionsExt;
        let file = capture.join(format!("{}.json", stored.id.0));
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(&capture).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
}
