//! `/constraints` 명령의 상태: 마지막으로 읽은 제약 목록과 그 revision을 들고, 변경 요청을 만든다.
//! 전체 화면과 plain 출력이 같이 쓴다. 변경은 사용자가 목록에서 본 revision을 싣고 가므로, 그동안 바뀌었으면 engine이
//! 거절하고 이어 보낸 목록 조회가 새 목록을 보인다.
//! 설계: docs/design/constraints.md#되돌리기, docs/design/tui.md

use std::collections::VecDeque;

use saturn_protocol::ids::ChatId;
use saturn_protocol::rpc::{
    ConstraintChangeInfo, ConstraintInfo, ConstraintStatus, QueryResult, Request,
};

/// `/constraints` 명령의 동작. 번호는 목록에 적힌 제약 번호와 변경 번호다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ConstraintsAction {
    /// 후보와 유효 제약을 보인다.
    List,
    /// 변경 내역을 보인다.
    History,
    /// 원문 그대로 등록한다. 공백을 포함할 수 있어 `add` 뒤 나머지 전체다.
    Add { text: String },
    /// 영구 해제.
    Release { id: u64 },
    /// 제약이 아니었다는 뜻의 잘못 등록 해제.
    Mistaken { id: u64 },
    /// 변경 내역의 한 건을 되돌린다.
    Undo { event: u64 },
}

/// 읽어 온 목록이 어느 줄로 그려지는지.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ListView {
    Constraints,
    History,
}

/// 채팅의 제약과 변경 내역을 읽은 때의 모습.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Listing {
    pub(crate) chat: ChatId,
    pub(crate) revision: u64,
    pub(crate) constraints: Vec<ConstraintInfo>,
    pub(crate) changes: Vec<ConstraintChangeInfo>,
}

/// 요청을 만들지 못한 까닭. 요청은 보내지 않는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeskError {
    /// 목록을 본 적이 없어 revision을 모른다.
    ListFirst,
    /// 목록에 없는 번호.
    Unknown(u64),
    /// 유효 제약이 아니라 해제할 수 없다.
    NotActive(u64),
    /// 가장 최근 변경이 아니거나 되돌릴 수 없는 종류다.
    NotUndoable(u64),
}

#[derive(Debug, Default)]
pub(crate) struct Desk {
    listing: Option<Listing>,
    /// 보낸 `ListConstraints`가 답으로 그려질 모양. 보낸 순서대로 온다.
    views: VecDeque<ListView>,
}

impl Desk {
    // cost: time O(n), heap O(1), stack O(1)
    // vars: n = 제약 수와 변경 수
    // basis: estimate
    /// 동작을 요청으로 바꾼다. 목록을 본 revision이 필요한 변경은 목록을 보기 전에는 보내지 않는다. 변경 요청 뒤에는
    /// 언제나 목록 조회가 따라가 결과를 보이고 revision을 새로 읽는다.
    ///
    /// # Errors
    /// 목록을 본 적이 없거나 번호가 맞지 않으면 `DeskError`.
    pub(crate) fn requests(
        &mut self,
        chat: ChatId,
        action: ConstraintsAction,
    ) -> Result<Vec<Request>, DeskError> {
        let list = Request::ListConstraints { chat };
        let (first, view) = match action {
            ConstraintsAction::List => return Ok(self.list_only(list, ListView::Constraints)),
            ConstraintsAction::History => return Ok(self.list_only(list, ListView::History)),
            ConstraintsAction::Add { text } => (Request::AddConstraint { chat, text }, None),
            ConstraintsAction::Release { id } | ConstraintsAction::Mistaken { id } => {
                let mistaken = matches!(action, ConstraintsAction::Mistaken { .. });
                let listing = self.seen(chat)?;
                let found = listing
                    .constraints
                    .iter()
                    .find(|constraint| constraint.id.0 == id)
                    .ok_or(DeskError::Unknown(id))?;
                if found.status != ConstraintStatus::Active {
                    return Err(DeskError::NotActive(id));
                }
                let release = Request::ReleaseConstraint {
                    constraint: found.id,
                    revision: listing.revision,
                    mistaken,
                };
                (release, None)
            }
            ConstraintsAction::Undo { event } => {
                let listing = self.seen(chat)?;
                let found = listing
                    .changes
                    .iter()
                    .find(|change| change.event == event)
                    .ok_or(DeskError::Unknown(event))?;
                if !found.undoable {
                    return Err(DeskError::NotUndoable(event));
                }
                let undo = Request::UndoConstraintChange {
                    constraint: found.constraint,
                    event,
                    revision: listing.revision,
                };
                (undo, None)
            }
        };
        self.views.push_back(view.unwrap_or(ListView::Constraints));
        Ok(vec![first, list])
    }

    fn list_only(&mut self, list: Request, view: ListView) -> Vec<Request> {
        self.views.push_back(view);
        vec![list]
    }

    fn seen(&self, chat: ChatId) -> Result<&Listing, DeskError> {
        self.listing
            .as_ref()
            .filter(|listing| listing.chat == chat)
            .ok_or(DeskError::ListFirst)
    }

    /// 조회 결과가 제약 목록이면 기억하고 그릴 모양과 함께 돌려준다. 다른 결과면 `None`이다.
    pub(crate) fn on_result(&mut self, result: QueryResult) -> Option<(ListView, Listing)> {
        let QueryResult::Constraints {
            chat,
            revision,
            constraints,
            changes,
        } = result
        else {
            return None;
        };
        let listing = Listing {
            chat,
            revision,
            constraints,
            changes,
        };
        self.listing = Some(listing.clone());
        let view = self.views.pop_front().unwrap_or(ListView::Constraints);
        Some((view, listing))
    }
}
