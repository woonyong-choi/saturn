//! 보내기 전 입력에 대한 사용자 요청: 새 작업으로 보내기, 바로 보내기, 취소.
//! 설계: docs/design/input-handling.md

use saturn_core::queue::{QueueError, QueuedInput};
use saturn_core::routers::calibration::Signal;
use saturn_protocol::ids::InputId;
use saturn_protocol::state::{Disposition, InputState};

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

    /// 대기 입력을 router에 묻지 않고 실행 중인 작업에 끼워 넣는다. 끼워 넣을 수 없으면(실행 중인 작업이 없거나
    /// provider가 끼워 넣기를 아직 지원하지 않으면) 같은 채팅 대기열 맨 앞에서 다음 차례를 기다린다.
    ///
    /// # Errors
    /// `run_as_new_task`와 같다.
    pub(crate) async fn send_now(
        &mut self,
        client: ClientId,
        input: InputId,
    ) -> Result<(), EngineError> {
        let record = self.attached_input(client, input)?;
        let previous = self.queue.send_now(input)?;
        if previous != Some(Disposition::Steer) {
            self.note_user_override(input, Signal::Missed);
        }
        self.notify_input(input).await;
        self.advance(record.chat).await;
        Ok(())
    }

    /// 끼워 넣기를 받지 않은 충돌 입력에 대한 사용자의 답. 멈추고 실행은 기존 멈춤 규칙을 쓴 뒤 그 입력을 재개하고,
    /// 대기는 맨 앞 대기를 그대로 둔다. engine은 이 답 없이 멈추지 않는다.
    ///
    /// # Errors
    /// `run_as_new_task`와 같고, 묻는 중이 아니면(다른 TUI가 먼저 답했거나 이미 다음 차례로 갔으면) `Queue(NotAwaitingStop)`.
    pub(crate) async fn answer_stop_confirm(
        &mut self,
        client: ClientId,
        input: InputId,
        stop: bool,
    ) -> Result<(), EngineError> {
        let record = self.attached_input(client, input)?;
        if !self.queue.awaits_stop(input) {
            return Err(QueueError::NotAwaitingStop(input).into());
        }
        if !stop {
            self.queue.keep_waiting(input)?;
            self.notify_input(input).await;
            self.advance(record.chat).await;
            return Ok(());
        }
        self.flow.run_after_stop.push(input);
        let stopped = self.stop_chat(record.chat).await;
        if stopped.is_err() {
            self.flow.run_after_stop.retain(|queued| *queued != input);
        }
        stopped
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
            .routed
            .get(&input)
            .and_then(|routed| routed.judgment);
        if let Some(judgment) = judgment {
            self.note_reaction(judgment, signal);
        }
    }
}
