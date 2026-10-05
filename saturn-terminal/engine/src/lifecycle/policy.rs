//! 판단 정책 고정 테스트(#537): 피드백, 취소, 실패가 정책을 바꾸지 않고, 입력은 접수 때 정한 설정 번호의 정책으로만 판단한다.

use std::time::Instant;

use saturn_core::routers::calibration::Signal;
use saturn_protocol::ids::{InputId, SettingsRevision};

use super::crash_recovery::Restarted;
use super::support::{CLIENT, Flow, idle_reply, router_down};
use crate::outcomes::OBSERVE_WINDOW;
use crate::store::test_judgment;

const STRICT: &str = "[router.thresholds]\nkeep_current = 0.85\n";
const STRICTER: &str = "[router.thresholds]\nkeep_current = 0.9\n";

fn keep_current_of(flow: &Flow, input: InputId) -> f64 {
    let thresholds = &flow.engine.flow.unrecorded[&input].context.thresholds;
    thresholds
        .iter()
        .find(|(name, _)| name == "keep_current")
        .map(|(_, value)| *value)
        .expect("keep_current should be recorded")
}

async fn change_settings(flow: &mut Flow, config: &str) -> SettingsRevision {
    flow.fixture.write_user_config(config);
    let workdir = flow.fixture.workdir.clone();
    flow.engine
        .apply_changed_settings(&[CLIENT], flow.chat, &workdir)
        .await
        .unwrap()
}

// 피드백, 취소, 반응 신호, router 실패를 100건 넣어도 활성 정책 지문, 기준값, router 모델, 설정 번호가 그대로다
#[tokio::test]
async fn feedback_cancel_and_failures_leave_the_active_policy_unchanged() {
    let mut flow = Flow::with_config(STRICT, router_down()).await;
    let revision = flow.engine.settings.current().unwrap();
    let digest = flow.engine.policy_digest_at(revision).await.unwrap();
    let model = flow.engine.routers.active().model().to_owned();
    let thresholds = flow
        .engine
        .settings
        .at(&flow.engine.store, revision)
        .await
        .unwrap()
        .thresholds();
    flow.submit("fix the build").await;
    let start = Instant::now();

    for _ in 0..100 {
        let id = flow
            .engine
            .store
            .record_judgment(&{
                let mut judgment = test_judgment(flow.chat);
                judgment.asked_with = Some(0.2);
                judgment
            })
            .await
            .unwrap()
            .unwrap();
        flow.engine.watch_judgment(flow.chat, id, start);
        flow.engine.note_reaction(id, Signal::Wrong);
        flow.engine.answer_feedback(id, false).await.unwrap();
        flow.engine.stop_chat(flow.chat).await.unwrap();
        flow.engine
            .settle_signals(start + OBSERVE_WINDOW)
            .await
            .unwrap();
    }

    assert_eq!(flow.engine.settings.current().unwrap(), revision);
    assert_eq!(flow.engine.settings.latest_of(flow.chat).unwrap(), revision);
    assert_eq!(
        flow.engine.policy_digest_at(revision).await.unwrap(),
        digest
    );
    assert_eq!(flow.engine.routers.active().model(), model);
    let after = flow
        .engine
        .settings
        .at(&flow.engine.store, revision)
        .await
        .unwrap()
        .thresholds();
    assert_eq!(after.keep_current, thresholds.keep_current);
    assert!(
        flow.engine
            .store
            .settings_snapshot(SettingsRevision(revision.0 + 1))
            .await
            .is_err()
    );
}

// 정책 교체 중 접수된 입력은 옛 번호와 새 번호가 섞이지 않고, 옛 설정으로 되돌리면 옛 번호를 다시 쓰며, 재시작해도 입력의 번호가 같다
#[tokio::test]
async fn inputs_keep_the_policy_they_were_accepted_under_across_swap_rollback_and_restart() {
    let mut flow = Flow::with_config(STRICT, vec![idle_reply(0.95), idle_reply(0.95)]).await;
    let old = flow.engine.settings.current().unwrap();
    let before = flow.accept_at("old policy input", old).await;
    let new = change_settings(&mut flow, STRICTER).await;
    assert_ne!(new, old);
    let after = flow.accept_at("new policy input", new).await;

    let decided_before = flow.router_now(before, false).await;
    let decided_after = flow.router_now(after, false).await;

    assert_eq!(decided_before.settings, old);
    assert_eq!(decided_after.settings, new);
    assert_eq!(keep_current_of(&flow, before), 0.85);
    assert_eq!(keep_current_of(&flow, after), 0.9);
    let digest_old = flow.engine.policy_digest_at(old).await.unwrap();
    let digest_new = flow.engine.policy_digest_at(new).await.unwrap();
    assert_ne!(digest_old, digest_new);

    let rolled_back = change_settings(&mut flow, STRICT).await;
    assert_eq!(rolled_back, old);

    let restarted = Restarted::after_shutdown(flow).await;
    assert_eq!(restarted.engine.queue.input(before).unwrap().settings, old);
    assert_eq!(restarted.engine.queue.input(after).unwrap().settings, new);
    assert_eq!(restarted.engine.settings.current().unwrap(), old);
    assert_eq!(
        restarted.engine.policy_digest_at(new).await.unwrap(),
        digest_new
    );
}
