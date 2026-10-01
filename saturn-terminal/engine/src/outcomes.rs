//! 판단 뒤 관찰 시간을 재고, 지나면 결과 신호를 판단 기록에 확정한다.
//! 설계: docs/design/judge-training.md

use std::time::{Duration, Instant};

use saturn_core::judges::calibration::{AskedAnswer, Signal};
use saturn_protocol::ids::{ChatId, JudgmentId};

use crate::store::StoreError;
use crate::{Engine, EngineError};

/// 판단 뒤 이만큼 입력이 들어오면 관찰이 끝난다.
pub(crate) const OBSERVE_INPUTS: u32 = 3;

/// 입력이 적어도 판단 뒤 이만큼 지나면 관찰이 끝난다.
pub(crate) const OBSERVE_WINDOW: Duration = Duration::from_secs(10 * 60);

/// 관찰 시간이 지난 판단을 찾는 주기. 초안 값.
pub(crate) const SETTLE_TICK: Duration = Duration::from_secs(30);

#[derive(Debug)]
struct Watched {
    judgment: JudgmentId,
    chat: ChatId,
    since: Instant,
    /// 판단 뒤 같은 채팅에 접수한 입력 수.
    inputs: u32,
    /// 관찰 중 처음 본 사용자 반응. 관찰이 끝나기 전에는 기록하지 않는다.
    reaction: Option<Signal>,
}

impl Watched {
    fn is_over(&self, now: Instant) -> bool {
        self.inputs >= OBSERVE_INPUTS || now.saturating_duration_since(self.since) >= OBSERVE_WINDOW
    }

    fn signal(&self) -> Signal {
        self.reaction.unwrap_or(Signal::Unconfirmed)
    }
}

/// 신호를 아직 확정하지 않은 판단. 메모리에만 두므로 engine이 다시 시작하면 관찰 중이던 판단의 신호는 비어 있는 채로 남는다.
#[derive(Debug, Default)]
pub(crate) struct SignalWatch {
    pending: Vec<Watched>,
}

impl SignalWatch {
    /// 판단을 기록한 직후 부른다. 이 판단을 부른 입력은 세지 않으므로 그 입력의 `note_input`은 이 호출 앞에 둔다.
    pub(crate) fn watch(&mut self, chat: ChatId, judgment: JudgmentId, now: Instant) {
        self.pending.push(Watched {
            judgment,
            chat,
            since: now,
            inputs: 0,
            reaction: None,
        });
    }

    /// 채팅에 새 입력을 접수할 때 그 채팅의 관찰 중인 판단마다 센다.
    pub(crate) fn note_input(&mut self, chat: ChatId) {
        for watched in self
            .pending
            .iter_mut()
            .filter(|watched| watched.chat == chat)
        {
            watched.inputs += 1;
        }
    }

    /// 사용자가 행동을 뒤집거나 취소했거나(`Wrong`) 같은 행동을 직접 했을 때(`Missed`) 부른다. 처음 반응만 남기고, 관찰 중이 아닌 판단은 무시한다.
    pub(crate) fn note_reaction(&mut self, judgment: JudgmentId, signal: Signal) {
        let Some(watched) = self
            .pending
            .iter_mut()
            .find(|watched| watched.judgment == judgment)
        else {
            return;
        };
        if watched.reaction.is_none() && signal != Signal::Unconfirmed {
            watched.reaction = Some(signal);
        }
    }

    /// 관찰 시간이 지난 판단과 확정할 신호. 반응이 없었으면 `Unconfirmed`다.
    pub(crate) fn settled(&self, now: Instant) -> Vec<(JudgmentId, Signal)> {
        self.pending
            .iter()
            .filter(|watched| watched.is_over(now))
            .map(|watched| (watched.judgment, watched.signal()))
            .collect()
    }

    pub(crate) fn forget(&mut self, judgment: JudgmentId) {
        self.pending.retain(|watched| watched.judgment != judgment);
    }

    pub(crate) fn is_watching(&self, judgment: JudgmentId) -> bool {
        self.pending
            .iter()
            .any(|watched| watched.judgment == judgment)
    }
}

impl Engine {
    // cost: time O(p), heap O(p), stack O(1), io p
    // vars: p = 관찰 중인 판단 수
    // basis: estimate
    /// 관찰 시간이 지난 판단의 신호를 기록 저장소에 쓰고 관찰을 끝낸다. 지워진 판단은 그냥 끝낸다.
    ///
    /// # Errors
    /// 쓰기 실패면 `Store`이고 그 판단은 다음 호출에서 다시 쓴다.
    pub(crate) async fn settle_signals(&mut self, now: Instant) -> Result<(), EngineError> {
        for (judgment, signal) in self.signals.settled(now) {
            match self.store.record_signal(judgment, signal).await {
                Ok(()) | Err(StoreError::NotFound { .. }) => self.signals.forget(judgment),
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    /// 판단 기록 직후 호출한다.
    pub(crate) fn watch_judgment(&mut self, chat: ChatId, judgment: JudgmentId, now: Instant) {
        self.signals.watch(chat, judgment, now);
    }

    /// 입력을 접수한 뒤 호출한다. 이 입력으로 관찰이 끝난 판단은 바로 확정한다.
    ///
    /// # Errors
    /// `settle_signals`와 같다.
    pub(crate) async fn note_input_accepted(
        &mut self,
        chat: ChatId,
        now: Instant,
    ) -> Result<(), EngineError> {
        self.signals.note_input(chat);
        self.settle_signals(now).await
    }

    pub(crate) fn note_reaction(&mut self, judgment: JudgmentId, signal: Signal) {
        self.signals.note_reaction(judgment, signal);
    }

    /// 사용자가 판단이 맞았는지 답한 것을 판단 기록에 쓴다.
    ///
    /// # Errors
    /// 없는 판단이거나 묻지 않은 판단이면 `Store(NotFound)`.
    pub(crate) async fn answer_feedback(
        &mut self,
        judgment: JudgmentId,
        correct: bool,
    ) -> Result<(), EngineError> {
        let answer = if correct {
            AskedAnswer::Correct
        } else {
            AskedAnswer::Wrong
        };
        Ok(self.store.record_asked_answer(judgment, answer).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHAT: ChatId = ChatId(1);
    const JUDGMENT: JudgmentId = JudgmentId(7);

    fn watching(now: Instant) -> SignalWatch {
        let mut watch = SignalWatch::default();
        watch.watch(CHAT, JUDGMENT, now);
        watch
    }

    #[test]
    fn settled_before_window_and_inputs_is_empty() {
        let start = Instant::now();
        let mut watch = watching(start);
        watch.note_reaction(JUDGMENT, Signal::Wrong);
        watch.note_input(CHAT);
        watch.note_input(CHAT);

        let settled = watch.settled(start + OBSERVE_WINDOW - Duration::from_secs(1));

        assert!(settled.is_empty());
    }

    #[test]
    fn settled_after_three_inputs_returns_reaction() {
        let start = Instant::now();
        let mut watch = watching(start);
        watch.note_reaction(JUDGMENT, Signal::Wrong);
        for _ in 0..OBSERVE_INPUTS {
            watch.note_input(CHAT);
        }

        let settled = watch.settled(start);

        assert_eq!(settled, vec![(JUDGMENT, Signal::Wrong)]);
    }

    #[test]
    fn settled_after_ten_minutes_without_reaction_is_unconfirmed() {
        let start = Instant::now();
        let watch = watching(start);

        let settled = watch.settled(start + OBSERVE_WINDOW);

        assert_eq!(settled, vec![(JUDGMENT, Signal::Unconfirmed)]);
    }

    #[test]
    fn note_reaction_keeps_first_reaction() {
        let start = Instant::now();
        let mut watch = watching(start);
        watch.note_reaction(JUDGMENT, Signal::Missed);

        watch.note_reaction(JUDGMENT, Signal::Wrong);

        let settled = watch.settled(start + OBSERVE_WINDOW);
        assert_eq!(settled, vec![(JUDGMENT, Signal::Missed)]);
    }

    #[test]
    fn note_input_in_other_chat_does_not_count() {
        let start = Instant::now();
        let mut watch = watching(start);

        for _ in 0..OBSERVE_INPUTS {
            watch.note_input(ChatId(2));
        }

        assert!(watch.settled(start).is_empty());
    }
}
