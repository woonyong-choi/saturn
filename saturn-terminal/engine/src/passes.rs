//! 하위 접속 출입증 관문. 연결 작업이 요청 처리 루프를 거치지 않고 출입증 확인, 상한 검사, 대기를 하게 하는 공유 상태다.
//! 판단은 `saturn_core::passes::PassTable`이 하고, 이 파일은 잠금, 대기 요청 깨우기, 토큰 만들기를 맡는다.
//! 설계: docs/design/child-sessions.md#요청-병렬-처리

use std::collections::HashMap;
use std::io::Read;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use saturn_core::passes::{
    Admission, Ended, Grant, PassLimits, PassTable, PassToken, Reject, Waiter,
};
use saturn_core::permission::Mode;
use saturn_protocol::ids::ChatId;
use tokio::sync::oneshot;

/// 출입증 토큰의 앞부분. 로그와 오류에서 이 모양의 글자를 가린다.
pub(crate) const TOKEN_PREFIX: &str = "saturn-pass-";

const TOKEN_BYTES: usize = 32;

/// 기다리던 요청의 결과.
#[derive(Debug)]
pub(crate) enum Waited {
    Granted(Grant),
    /// 부모가 멈추거나 끝나 더 기다릴 수 없다.
    Cancelled,
}

/// `PassGate::request`의 결과.
#[derive(Debug)]
pub(crate) enum Entry {
    Rejected(Reject),
    Admitted(Grant),
    Queued {
        position: u32,
        waiter: Waiter,
        wait: oneshot::Receiver<Waited>,
    },
}

#[derive(Debug)]
struct State {
    table: PassTable,
    waiting: HashMap<Waiter, oneshot::Sender<Waited>>,
    next_waiter: u64,
}

/// 연결 작업과 요청 처리 루프가 함께 쓴다. 잠금은 짧은 메모리 조회와 갱신에만 쥐고 `await` 사이에는 쥐지 않는다.
#[derive(Debug, Clone)]
pub(crate) struct PassGate {
    state: Arc<Mutex<State>>,
}

impl PassGate {
    pub(crate) fn new(limits: PassLimits) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                table: PassTable::new(limits),
                waiting: HashMap::new(),
                next_waiter: 1,
            })),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// 출입증을 확인하고 상한을 검사한다. 상한이 차면 대기열에 서고 자리가 나면 `wait`로 알려 준다.
    pub(crate) fn request(&self, token: &str, wanted: Option<Mode>) -> Entry {
        let mut state = self.lock();
        let waiter = Waiter(state.next_waiter);
        state.next_waiter += 1;
        let token = PassToken::new(token.to_owned());
        match state.table.request(waiter, &token, wanted) {
            Admission::Rejected(reject) => Entry::Rejected(reject),
            Admission::Admitted(grant) => Entry::Admitted(grant),
            Admission::Queued { position } => {
                let (sender, wait) = oneshot::channel();
                state.waiting.insert(waiter, sender);
                Entry::Queued {
                    position,
                    waiter,
                    wait,
                }
            }
        }
    }

    /// 기다리던 요청이 끊겼다.
    pub(crate) fn cancel(&self, waiter: Waiter) {
        let mut state = self.lock();
        if state.table.cancel(waiter) {
            state.waiting.remove(&waiter);
        }
    }

    /// 채팅에 출입증이 없으면 새로 만든다. 이미 있으면 그 토큰이고 모드만 갱신한다.
    ///
    /// # Errors
    /// 운영체제 난수를 읽지 못하면 오류.
    pub(crate) fn open_root(&self, chat: ChatId, mode: Mode) -> std::io::Result<PassToken> {
        let mut state = self.lock();
        if let Some(token) = state.table.token_of(chat).cloned() {
            state.table.set_mode(chat, mode);
            return Ok(token);
        }
        let token = new_token()?;
        state.table.open_root(chat, mode, token.clone());
        Ok(token)
    }

    /// 허용받은 하위 채팅과 그 출입증을 묶는다.
    ///
    /// # Errors
    /// 운영체제 난수를 읽지 못하면 오류. 이때 자리는 돌려준다.
    pub(crate) fn bind(&self, grant: &Grant, chat: ChatId) -> std::io::Result<PassToken> {
        let token = match new_token() {
            Ok(token) => token,
            Err(error) => {
                self.abandon(grant);
                return Err(error);
            }
        };
        self.lock().table.bind(grant, chat, token.clone());
        Ok(token)
    }

    /// 채팅을 만들지 못해 자리를 돌려준다.
    pub(crate) fn abandon(&self, grant: &Grant) {
        let mut state = self.lock();
        let ended = state.table.abandon(grant);
        finish(&mut state, ended);
    }

    /// 하위 접속 하나가 끝났다. 함께 멈춰야 하는 하위 채팅을 돌려준다.
    pub(crate) fn end(&self, chat: ChatId) -> Vec<ChatId> {
        let mut state = self.lock();
        let ended = state.table.end(chat);
        finish(&mut state, ended)
    }

    /// 채팅의 모든 하위 접속을 회수한다. 채팅 자신의 출입증은 그대로다.
    pub(crate) fn end_children(&self, chat: ChatId) -> Vec<ChatId> {
        let mut state = self.lock();
        let ended = state.table.end_children(chat);
        finish(&mut state, ended)
    }

    pub(crate) fn set_mode(&self, chat: ChatId, mode: Mode) {
        self.lock().table.set_mode(chat, mode);
    }

    /// 하위 채팅이 부모 모드를 넘는 모드로 바꾸려 하면 부모 모드를 돌려준다.
    pub(crate) fn check_mode(&self, chat: ChatId, mode: Mode) -> Result<(), Mode> {
        self.lock().table.check_mode(chat, mode)
    }

    pub(crate) fn parent_of(&self, chat: ChatId) -> Option<ChatId> {
        self.lock().table.parent_of(chat)
    }

    #[cfg(test)]
    pub(crate) fn depth_of(&self, chat: ChatId) -> Option<u32> {
        self.lock().table.depth_of(chat)
    }

    #[cfg(test)]
    pub(crate) fn mode_of(&self, chat: ChatId) -> Option<Mode> {
        self.lock().table.mode_of(chat)
    }

    #[cfg(test)]
    pub(crate) fn token_of(&self, chat: ChatId) -> Option<PassToken> {
        self.lock().table.token_of(chat).cloned()
    }

    #[cfg(test)]
    pub(crate) fn running_total(&self) -> u32 {
        self.lock().table.running_total()
    }

    #[cfg(test)]
    pub(crate) fn queued_total(&self) -> u32 {
        self.lock().table.queued_total()
    }
}

// cost: time O(p), heap O(p), stack O(1)
// vars: p = 자리가 난 대기 요청 수
// basis: estimate
/// 자리가 난 요청을 깨우고 취소된 요청에 알린다. 깨울 곳이 이미 없으면 그 자리를 돌려주고 다음 요청을 깨운다.
/// 함께 멈춰야 하는 하위 채팅을 돌려준다.
fn finish(state: &mut State, ended: Ended) -> Vec<ChatId> {
    let Ended {
        chats,
        cancelled,
        promoted,
    } = ended;
    for waiter in cancelled {
        if let Some(sender) = state.waiting.remove(&waiter) {
            let _ = sender.send(Waited::Cancelled); // 기다리던 연결이 이미 끊겼다
        }
    }
    let mut chats = chats;
    let mut pending = promoted;
    while let Some(promoted) = pending.pop() {
        let Some(sender) = state.waiting.remove(&promoted.waiter) else {
            continue;
        };
        if let Err(Waited::Granted(grant)) = sender.send(Waited::Granted(promoted.grant)) {
            // 깨울 곳이 사라졌으니 자리를 돌려주고 다음 요청에 준다
            let next = state.table.abandon(&grant);
            chats.extend(next.chats);
            pending.extend(next.promoted);
            for waiter in next.cancelled {
                if let Some(sender) = state.waiting.remove(&waiter) {
                    let _ = sender.send(Waited::Cancelled); // 기다리던 연결이 이미 끊겼다
                }
            }
        }
    }
    chats
}

// cost: time O(1), heap O(1), stack O(1), io 1
// basis: estimate
/// 운영체제 난수 32바이트의 16진수. 키가 아니고 이 engine이 사는 동안과 채팅 범위에서만 쓴다.
fn new_token() -> std::io::Result<PassToken> {
    let mut bytes = [0_u8; TOKEN_BYTES];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    let mut text = String::from(TOKEN_PREFIX);
    for byte in bytes {
        text.push_str(&format!("{byte:02x}"));
    }
    Ok(PassToken::new(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: ChatId = ChatId(1);

    fn gate(limits: PassLimits) -> (PassGate, PassToken) {
        let gate = PassGate::new(limits);
        let token = gate.open_root(ROOT, Mode::Edit).unwrap();
        (gate, token)
    }

    fn limits(max_concurrent: u32) -> PassLimits {
        PassLimits {
            max_depth: 3,
            max_concurrent,
            max_total: 32,
        }
    }

    #[test]
    fn token_has_the_prefix_and_is_new_every_time() {
        let first = new_token().unwrap();
        let second = new_token().unwrap();

        assert!(first.as_str().starts_with(TOKEN_PREFIX));
        assert_eq!(first.as_str().len(), TOKEN_PREFIX.len() + TOKEN_BYTES * 2);
        assert_ne!(first, second);
    }

    #[test]
    fn open_root_returns_the_same_token_for_the_same_chat() {
        let (gate, token) = gate(limits(2));

        assert_eq!(gate.open_root(ROOT, Mode::Ask).unwrap(), token);
        assert_eq!(gate.mode_of(ROOT), Some(Mode::Ask));
    }

    #[test]
    fn queued_request_wakes_when_a_place_frees() {
        let (gate, token) = gate(limits(1));
        let Entry::Admitted(first) = gate.request(token.as_str(), None) else {
            panic!("expected the first request to be admitted");
        };
        let child = gate.bind(&first, ChatId(2)).unwrap();
        let Entry::Queued { mut wait, .. } = gate.request(token.as_str(), None) else {
            panic!("expected the second request to wait");
        };
        assert!(wait.try_recv().is_err());

        gate.end(ChatId(2));

        assert!(matches!(wait.try_recv(), Ok(Waited::Granted(_))));
        assert!(gate.token_of(ChatId(2)).is_none());
        assert_ne!(child, token);
    }

    #[test]
    fn dropped_waiter_passes_its_place_on_to_the_next() {
        let (gate, token) = gate(limits(1));
        let Entry::Admitted(first) = gate.request(token.as_str(), None) else {
            panic!("expected the first request to be admitted");
        };
        gate.bind(&first, ChatId(2)).unwrap();
        let Entry::Queued { wait: dropped, .. } = gate.request(token.as_str(), None) else {
            panic!("expected the second request to wait");
        };
        let Entry::Queued { mut wait, .. } = gate.request(token.as_str(), None) else {
            panic!("expected the third request to wait");
        };
        drop(dropped);

        gate.end(ChatId(2));

        assert!(matches!(wait.try_recv(), Ok(Waited::Granted(_))));
        assert_eq!(gate.running_total(), 1);
    }

    #[test]
    fn ending_children_cancels_queued_requests() {
        let (gate, token) = gate(limits(1));
        let Entry::Admitted(first) = gate.request(token.as_str(), None) else {
            panic!("expected the first request to be admitted");
        };
        gate.bind(&first, ChatId(2)).unwrap();
        let Entry::Queued { mut wait, .. } = gate.request(token.as_str(), None) else {
            panic!("expected the second request to wait");
        };

        let stopped = gate.end_children(ROOT);

        assert_eq!(stopped, vec![ChatId(2)]);
        assert!(matches!(wait.try_recv(), Ok(Waited::Cancelled)));
    }

    #[test]
    fn token_debug_never_shows_the_text() {
        let (gate, token) = gate(limits(1));

        assert!(!format!("{gate:?}").contains(token.as_str()));
    }
}
