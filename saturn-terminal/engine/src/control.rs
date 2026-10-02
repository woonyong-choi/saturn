//! 보내기 전 입력에 대한 사용자 요청: 새 작업으로 보내기, 바로 보내기, 취소.
//! 설계: docs/design/input-handling.md

use saturn_core::judges::calibration::Signal;
use saturn_core::queue::QueuedInput;
use saturn_protocol::ids::{ChatRevision, InputId};
use saturn_protocol::rpc::Alert;
use saturn_protocol::state::{Disposition, InputState};

use crate::flow::JudgeKind;
use crate::intake::Verdict;
use crate::rpc::ClientId;
use crate::{Engine, EngineError};

impl Engine {
    /// 아직 보내지 않은 입력을 새 작업으로 보낸다.
    ///
    /// # Errors
    /// 붙지 않은 채팅의 입력이면 `ChatNotAttached`, 대기 중이 아니거나 이미 내준 입력이면 `Queue`.
    pub(crate) async fn run_as_new_task(
        &mut self,
        client: ClientId,
        input: InputId,
    ) -> Result<(), EngineError> {
        let record = self.attached_input(client, input)?;
        self.queue.redirect(input, Disposition::NewTask)?;
        self.note_user_override(input, Signal::Missed);
        self.notify_input(input).await;
        self.advance(record.chat).await;
        Ok(())
    }

    /// 대기 입력을 judge에 한 번 물어 끼워 넣기나 새 작업으로 바로 보낸다. 대기로 답하거나 판단할 수 없으면
    /// 차례를 기다린다. judge가 실패하면 `판단기 연결 없음 · 차례에 보냅니다`를 보인다.
    /// TODO(#36): 판단이 이어 가는 입력을 뒤집는 관계(`conflicts`)로 답할 때 바로 멈출지
    ///
    /// # Errors
    /// `run_as_new_task`와 같다. 판단 호출 실패는 오류가 아니다.
    pub(crate) async fn send_now(
        &mut self,
        client: ClientId,
        input: InputId,
    ) -> Result<(), EngineError> {
        let record = self.attached_input(client, input)?;
        if record.state != InputState::Queued {
            return Err(saturn_core::queue::QueueError::InvalidTransition {
                from: record.state,
                to: InputState::Queued,
            }
            .into());
        }
        if record.pinned_model.is_none() {
            if self.flow.send_now_pending.insert(input) {
                let revision = self.queue.revision(record.chat);
                self.start_judge(&record, revision, JudgeKind::SendNow);
            }
            return Ok(());
        }
        self.notify_input(input).await;
        self.advance(record.chat).await;
        Ok(())
    }

    /// 바로 보내기 판단이 돌아왔다. 그사이 입력이 대기를 벗어났으면(보냈거나 취소했거나 멈췄으면) 판단은 기록만 한다.
    pub(crate) async fn finish_send_now(
        &mut self,
        input: InputId,
        revision: ChatRevision,
        verdict: Verdict,
    ) -> Result<(), EngineError> {
        let record = self.queued(input)?;
        let current = self.queue.revision(record.chat);
        if record.state != InputState::Queued || current != revision {
            self.settle_record(input, true).await;
            self.notify_input(input).await;
            return Ok(());
        }
        self.settle_record(input, false).await;
        if verdict.failed {
            self.notify_alert(record.chat, Alert::JudgeDownSendingInOrder)
                .await;
        } else {
            self.redirect_by_verdict(input, verdict.decision.disposition)?;
        }
        self.notify_input(input).await;
        Ok(())
    }

    /// 판단이 대기면 그대로 둔다.
    fn redirect_by_verdict(
        &mut self,
        input: InputId,
        disposition: Disposition,
    ) -> Result<(), EngineError> {
        if disposition == Disposition::Queue {
            return Ok(());
        }
        self.queue.redirect(input, disposition)?;
        self.note_user_override(input, Signal::Missed);
        Ok(())
    }

    /// 보내기 전 입력만 취소한다. `전달 중`과 `반영됨` 입력은 거절한다.
    ///
    /// # Errors
    /// 붙지 않은 채팅의 입력이면 `ChatNotAttached`, 이미 보냈으면 `Queue(AlreadySent)`, 기록 쓰기 실패면 `Store`.
    pub(crate) async fn cancel_input(
        &mut self,
        client: ClientId,
        input: InputId,
    ) -> Result<(), EngineError> {
        let record = self.attached_input(client, input)?;
        self.queue.cancel(input)?;
        self.store
            .set_input_state(input, InputState::Cancelled, None)
            .await?;
        self.note_user_override(input, Signal::Wrong);
        self.notify_input(input).await;
        self.advance(record.chat).await;
        Ok(())
    }

    fn attached_input(&self, client: ClientId, input: InputId) -> Result<QueuedInput, EngineError> {
        let record = self.queued(input)?;
        self.attached_workdir(client, record.chat)?;
        Ok(record)
    }

    /// 판단을 받은 입력을 사용자가 뒤집었다. 판단 기록이 없으면 알릴 곳이 없다.
    fn note_user_override(&mut self, input: InputId, signal: Signal) {
        let judgment = self
            .flow
            .judged
            .get(&input)
            .and_then(|judged| judged.judgment);
        if let Some(judgment) = judgment {
            self.note_reaction(judgment, signal);
        }
    }
}
