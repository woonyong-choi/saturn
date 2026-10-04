//! 하위 접속 출입증: 발급 범위, 깊이와 동시 상한, 대기열, 회수를 정한다. 파일, 네트워크, 프로세스는 다루지 않는다.
//! 토큰 글자는 호출하는 쪽이 만들어 넘기고, 깨우기와 연결 종료는 호출하는 쪽이 한다.
//! 설계: docs/design/child-sessions.md

use std::collections::{HashMap, VecDeque};

use saturn_protocol::ids::ChatId;

use crate::permission::Mode;

/// 출입증 토큰. router 키가 아니고 채팅 하나의 하위 접속에만 쓰며, 어떤 출력에도 글자가 나오지 않는다.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PassToken(String);

impl PassToken {
    pub fn new(text: String) -> Self {
        Self(text)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for PassToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PassToken(..)")
    }
}

/// 상한. 한 채팅이 동시에 거느리는 하위 접속 수와 engine 전체 수는 provider 프로세스 수를 막는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PassLimits {
    /// 사용자가 연 채팅이 0이고 그 하위가 1이다. 이 깊이를 넘는 하위 접속은 거절한다.
    pub max_depth: u32,
    /// 한 채팅이 동시에 거느리는 하위 접속 수. 넘으면 대기열에 둔다.
    pub max_concurrent: u32,
    /// engine 전체의 동시 하위 접속 수. 넘으면 대기열에 둔다.
    pub max_total: u32,
}

impl PassLimits {
    /// 부하 시험으로 정했다(docs/design/child-sessions.md#부하-시험). 동시 10개까지 engine 응답 시간 p95가 15ms 이하였다.
    pub const DEFAULT: Self = Self {
        max_depth: 2,
        max_concurrent: 5,
        max_total: 10,
    };
}

impl Default for PassLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// 요청 하나를 가리키는 번호. 호출하는 쪽이 정하고 대기열에서 요청을 찾는 데 쓴다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Waiter(pub u64);

/// 동시 상한 한 자리. 하위 접속이 끝나거나 회수될 때 돌려준다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Slot(pub u64);

/// 허용한 하위 접속의 범위.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    pub slot: Slot,
    /// 부모 채팅. 하위 접속은 이 채팅의 하위 작업으로만 붙는다.
    pub parent: ChatId,
    /// 하위 채팅의 깊이. 사용자가 연 채팅의 바로 아래가 1이다.
    pub depth: u32,
    /// 부모 모드를 넘지 않는 모드.
    pub mode: Mode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reject {
    /// 발급한 적 없거나 이미 회수된 출입증.
    UnknownPass,
    DepthExceeded {
        max: u32,
    },
    /// 요청한 모드가 부모 모드보다 크다.
    ModeAboveParent {
        parent: Mode,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Admission {
    Admitted(Grant),
    /// 상한이 차서 기다린다. `position`은 대기열에서 1부터 센 자리.
    Queued {
        position: u32,
    },
    Rejected(Reject),
}

/// 대기하다 자리가 난 요청.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Promoted {
    pub waiter: Waiter,
    pub grant: Grant,
}

/// 하위 접속을 끝내거나 회수한 결과. 호출하는 쪽이 `chats`를 멈추고, `cancelled`의 요청에 실패를 답하고,
/// `promoted`의 요청을 이어 간다.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Ended {
    /// 함께 끝나야 하는 하위 채팅. 깊은 쪽이 앞이다.
    pub chats: Vec<ChatId>,
    /// 부모가 사라져 더 기다릴 수 없는 대기 요청.
    pub cancelled: Vec<Waiter>,
    /// 비어 난 자리로 들어간 대기 요청. 대기열 순서다.
    pub promoted: Vec<Promoted>,
}

#[derive(Debug)]
struct Node {
    parent: Option<ChatId>,
    depth: u32,
    mode: Mode,
    token: PassToken,
    /// 이 채팅이 거느린 하위 채팅.
    children: Vec<ChatId>,
    /// 이 채팅이 쓰고 있는 동시 자리 수. 채팅이 만들어지기 전의 자리도 센다.
    running: u32,
    /// 이 채팅 자신이 부모에게서 받은 자리.
    slot: Option<Slot>,
}

#[derive(Debug)]
struct Pending {
    waiter: Waiter,
    parent: ChatId,
    wanted: Option<Mode>,
}

#[derive(Debug)]
struct SlotInfo {
    parent: ChatId,
    /// 채팅을 만든 뒤에 채운다.
    child: Option<ChatId>,
}

/// 출입증과 상한의 정본. 메모리에만 둔다.
#[derive(Debug)]
pub struct PassTable {
    limits: PassLimits,
    nodes: HashMap<ChatId, Node>,
    by_token: HashMap<PassToken, ChatId>,
    slots: HashMap<Slot, SlotInfo>,
    queue: VecDeque<Pending>,
    next_slot: u64,
}

impl PassTable {
    pub fn new(limits: PassLimits) -> Self {
        Self {
            limits,
            nodes: HashMap::new(),
            by_token: HashMap::new(),
            slots: HashMap::new(),
            queue: VecDeque::new(),
            next_slot: 1,
        }
    }

    pub fn limits(&self) -> PassLimits {
        self.limits
    }

    /// 상한을 바꾼다. 늘리면 대기 요청이 자리를 얻을 수 있다.
    pub fn set_limits(&mut self, limits: PassLimits) -> Ended {
        self.limits = limits;
        let mut ended = Ended::default();
        self.promote(&mut ended);
        ended
    }

    /// 사용자가 연 채팅에 출입증을 준다. 이미 있으면 모드만 갱신하고 `false`다.
    pub fn open_root(&mut self, chat: ChatId, mode: Mode, token: PassToken) -> bool {
        if let Some(node) = self.nodes.get_mut(&chat) {
            node.mode = mode;
            return false;
        }
        self.by_token.insert(token.clone(), chat);
        self.nodes.insert(
            chat,
            Node {
                parent: None,
                depth: 0,
                mode,
                token,
                children: Vec::new(),
                running: 0,
                slot: None,
            },
        );
        true
    }

    pub fn token_of(&self, chat: ChatId) -> Option<&PassToken> {
        self.nodes.get(&chat).map(|node| &node.token)
    }

    pub fn lookup(&self, token: &PassToken) -> Option<ChatId> {
        self.by_token.get(token).copied()
    }

    pub fn parent_of(&self, chat: ChatId) -> Option<ChatId> {
        self.nodes.get(&chat).and_then(|node| node.parent)
    }

    pub fn depth_of(&self, chat: ChatId) -> Option<u32> {
        self.nodes.get(&chat).map(|node| node.depth)
    }

    pub fn mode_of(&self, chat: ChatId) -> Option<Mode> {
        self.nodes.get(&chat).map(|node| node.mode)
    }

    /// 채팅의 모드를 갱신한다. 하위 채팅의 실제 모드는 판정 때 부모 모드와 비교해 낮춘다.
    pub fn set_mode(&mut self, chat: ChatId, mode: Mode) {
        if let Some(node) = self.nodes.get_mut(&chat) {
            node.mode = mode;
        }
    }

    /// 하위 채팅이면 부모 모드를 넘는 모드를 거절한다. 사용자가 연 채팅은 제한이 없다.
    ///
    /// # Errors
    /// 부모 모드보다 크면 부모 모드를 돌려준다.
    pub fn check_mode(&self, chat: ChatId, mode: Mode) -> Result<(), Mode> {
        let parent = self
            .nodes
            .get(&chat)
            .and_then(|node| node.parent)
            .and_then(|parent| self.nodes.get(&parent));
        match parent {
            Some(parent) if mode > parent.mode => Err(parent.mode),
            _ => Ok(()),
        }
    }

    /// 지금 도는 하위 접속 수. 채팅을 만들기 전의 자리도 센다.
    pub fn running_total(&self) -> u32 {
        u32::try_from(self.slots.len()).unwrap_or(u32::MAX)
    }

    pub fn queued_total(&self) -> u32 {
        u32::try_from(self.queue.len()).unwrap_or(u32::MAX)
    }

    pub fn running_of(&self, chat: ChatId) -> u32 {
        self.nodes.get(&chat).map_or(0, |node| node.running)
    }

    // cost: time O(q), heap O(1), stack O(1)
    // vars: q = 대기 요청 수
    // basis: estimate
    /// 출입증으로 하위 접속을 요청한다. 거절은 대기열에 넣지 않는다. 부모와 engine 전체에 자리가 있으면 바로 허용하고,
    /// 없으면 대기열 끝에 선다.
    pub fn request(
        &mut self,
        waiter: Waiter,
        token: &PassToken,
        wanted: Option<Mode>,
    ) -> Admission {
        let Some(parent) = self.by_token.get(token).copied() else {
            return Admission::Rejected(Reject::UnknownPass);
        };
        let Some(node) = self.nodes.get(&parent) else {
            return Admission::Rejected(Reject::UnknownPass);
        };
        if node.depth + 1 > self.limits.max_depth {
            return Admission::Rejected(Reject::DepthExceeded {
                max: self.limits.max_depth,
            });
        }
        if let Some(wanted) = wanted
            && wanted > node.mode
        {
            return Admission::Rejected(Reject::ModeAboveParent { parent: node.mode });
        }
        if self.has_room(parent) {
            return Admission::Admitted(self.take_slot(parent, wanted));
        }
        self.queue.push_back(Pending {
            waiter,
            parent,
            wanted,
        });
        Admission::Queued {
            position: self.queued_total(),
        }
    }

    /// 허용받은 자리에 만든 하위 채팅과 그 출입증을 묶는다. 하위 채팅도 자기 하위 접속을 받을 수 있다.
    pub fn bind(&mut self, grant: &Grant, chat: ChatId, token: PassToken) {
        let Some(info) = self.slots.get_mut(&grant.slot) else {
            return;
        };
        info.child = Some(chat);
        self.by_token.insert(token.clone(), chat);
        self.nodes.insert(
            chat,
            Node {
                parent: Some(grant.parent),
                depth: grant.depth,
                mode: grant.mode,
                token,
                children: Vec::new(),
                running: 0,
                slot: Some(grant.slot),
            },
        );
        if let Some(parent) = self.nodes.get_mut(&grant.parent) {
            parent.children.push(chat);
        }
    }

    /// 채팅을 만들지 못해 자리를 쓰지 않고 돌려준다.
    pub fn abandon(&mut self, grant: &Grant) -> Ended {
        let mut ended = Ended::default();
        self.free_slot(grant.slot);
        self.promote(&mut ended);
        ended
    }

    /// 기다리던 요청이 끊겼다. 대기열에서 뺀다.
    pub fn cancel(&mut self, waiter: Waiter) -> bool {
        let before = self.queue.len();
        self.queue.retain(|pending| pending.waiter != waiter);
        self.queue.len() != before
    }

    // cost: time O(n + q), heap O(n), stack O(1)
    // vars: n = 회수하는 하위 채팅 수, q = 대기 요청 수
    // basis: estimate
    /// 채팅의 모든 하위 접속을 회수한다. 출입증은 그대로 두어 다음 작업에서 다시 쓴다.
    pub fn end_children(&mut self, chat: ChatId) -> Ended {
        let mut ended = Ended::default();
        let children = self
            .nodes
            .get(&chat)
            .map(|node| node.children.clone())
            .unwrap_or_default();
        for child in children {
            self.remove_tree(child, true, &mut ended);
        }
        self.drop_waiting_under(&[chat], &mut ended);
        self.promote(&mut ended);
        ended
    }

    /// 채팅의 출입증을 지우고 그 아래 하위 접속을 모두 회수한다. `chats`에는 채팅 자신을 넣지 않는다.
    pub fn end(&mut self, chat: ChatId) -> Ended {
        let mut ended = Ended::default();
        self.remove_tree(chat, false, &mut ended);
        self.promote(&mut ended);
        ended
    }

    fn has_room(&self, parent: ChatId) -> bool {
        let under_parent = self.nodes.get(&parent).map_or(0, |node| node.running);
        under_parent < self.limits.max_concurrent && self.running_total() < self.limits.max_total
    }

    fn take_slot(&mut self, parent: ChatId, wanted: Option<Mode>) -> Grant {
        let slot = Slot(self.next_slot);
        self.next_slot += 1;
        self.slots.insert(
            slot,
            SlotInfo {
                parent,
                child: None,
            },
        );
        let node = self
            .nodes
            .get_mut(&parent)
            .expect("a slot is only taken under a registered parent");
        node.running += 1;
        Grant {
            slot,
            parent,
            depth: node.depth + 1,
            mode: wanted.map_or(node.mode, |wanted| wanted.min(node.mode)),
        }
    }

    fn free_slot(&mut self, slot: Slot) {
        let Some(info) = self.slots.remove(&slot) else {
            return;
        };
        if let Some(parent) = self.nodes.get_mut(&info.parent) {
            parent.running = parent.running.saturating_sub(1);
        }
    }

    /// 채팅과 그 아래를 지운다. 멈출 채팅은 깊은 쪽이 앞이 되도록 `ended.chats`에 쌓고, `include_self`가 참일 때만
    /// 채팅 자신도 넣는다.
    fn remove_tree(&mut self, chat: ChatId, include_self: bool, ended: &mut Ended) {
        let Some(node) = self.nodes.remove(&chat) else {
            return;
        };
        for child in node.children.clone() {
            self.remove_tree(child, true, ended);
        }
        self.by_token.remove(&node.token);
        if let Some(parent) = node.parent.and_then(|parent| self.nodes.get_mut(&parent)) {
            parent.children.retain(|other| *other != chat);
        }
        if let Some(slot) = node.slot {
            self.free_slot(slot);
        }
        // 이 채팅 밑에서 아직 채팅을 만들지 못한 자리도 함께 돌려준다
        let orphan: Vec<Slot> = self
            .slots
            .iter()
            .filter(|(_, info)| info.parent == chat)
            .map(|(slot, _)| *slot)
            .collect();
        for slot in orphan {
            self.slots.remove(&slot);
        }
        self.drop_waiting_under(&[chat], ended);
        if include_self {
            ended.chats.push(chat);
        }
    }

    fn drop_waiting_under(&mut self, parents: &[ChatId], ended: &mut Ended) {
        let (dropped, kept): (Vec<Pending>, Vec<Pending>) = std::mem::take(&mut self.queue)
            .into_iter()
            .partition(|pending| parents.contains(&pending.parent));
        self.queue = kept.into();
        ended
            .cancelled
            .extend(dropped.into_iter().map(|pending| pending.waiter));
    }

    // cost: time O(q), heap O(q), stack O(1)
    // vars: q = 대기 요청 수
    // basis: estimate
    /// 대기열 앞에서부터 자리가 있는 요청을 허용한다. 부모가 사라진 요청은 취소한다.
    /// 자리가 없는 요청은 건너뛰어, 다른 부모의 요청이 앞 요청에 막히지 않는다.
    fn promote(&mut self, ended: &mut Ended) {
        let mut waiting = std::mem::take(&mut self.queue);
        while let Some(pending) = waiting.pop_front() {
            if !self.nodes.contains_key(&pending.parent) {
                ended.cancelled.push(pending.waiter);
            } else if self.has_room(pending.parent) {
                let grant = self.take_slot(pending.parent, pending.wanted);
                ended.promoted.push(Promoted {
                    waiter: pending.waiter,
                    grant,
                });
            } else {
                self.queue.push_back(pending);
            }
        }
    }
}

#[cfg(test)]
mod tests;
