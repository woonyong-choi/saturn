//! 압축·전환 뒤 새 질문의 원문 복원과 provider 모드 경계.

use super::support::{Flow, idle_reply, text, turn_completed};
use crate::providers::test_support::{CLAUDE, CODEX, Call};

#[tokio::test]
async fn recall_reaches_provider_after_switch_without_changing_stored_input() {
    for mode in ["saturn", "provider"] {
        let config = format!("[context]\nmode = \"{mode}\"\n");
        let mut flow = Flow::with_config(&config, (0..4).map(|_| idle_reply(0.95)).collect()).await;
        let old = flow.add_provider(CODEX);
        flow.engine.switch_provider(flow.chat, CODEX);
        flow.submit("release region: ap-northeast-2").await;
        let agent = flow.agent();
        flow.event(CODEX, text(agent, "region recorded")).await;
        flow.event(CODEX, turn_completed(agent)).await;
        flow.engine.switch_provider(flow.chat, CLAUDE);
        flow.submit("begin review").await;
        let agent = flow.agent();
        flow.event(CLAUDE, turn_completed(agent)).await;
        flow.event(CLAUDE, text(agent, "review ready")).await;
        flow.event(CLAUDE, turn_completed(agent)).await;
        let query = "Which release region did I specify?";
        let input = flow.submit(query).await;
        let sent = flow
            .fake
            .calls()
            .into_iter()
            .filter_map(|call| match call {
                Call::SendTurn { text, .. } => Some(text),
                _ => None,
            })
            .next_back()
            .unwrap();
        assert_eq!(sent.contains("ap-northeast-2"), mode == "saturn", "{sent}");
        assert!(sent.ends_with(query));
        assert_eq!(flow.engine.queue.input(input).unwrap().text, query);
        assert_eq!(
            flow.engine.store.stored_input(input).await.unwrap().0.text,
            query
        );
        // 같은 session에서 다시 물어도 앞서 전달했다는 이유로 근거를 생략하지 않는다.
        flow.event(CLAUDE, text(agent, "region recalled")).await;
        flow.event(CLAUDE, turn_completed(agent)).await;
        flow.submit(query).await;
        let repeated = flow
            .fake
            .calls()
            .into_iter()
            .filter_map(|call| match call {
                Call::SendTurn { text, .. } => Some(text),
                _ => None,
            })
            .next_back()
            .unwrap();
        assert_eq!(repeated.contains("ap-northeast-2"), mode == "saturn");
        assert_eq!(
            old.calls()
                .iter()
                .filter(|call| matches!(call, Call::SendTurn { .. }))
                .count(),
            1
        );
    }
}
