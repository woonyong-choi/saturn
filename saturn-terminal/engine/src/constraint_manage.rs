//! 제약 관리: 사용자가 `/constraints`로 직접 등록하고, 목록과 변경 내역을 읽고, 변경 한 건을 되돌린다.
//! 새 저장소 없이 제약 표와 이벤트를 쓰고, 해제는 `release_constraint`를 쓴다.
//! 설계: docs/design/constraints.md#되돌리기

use saturn_core::constraints::scope_of;
use saturn_core::routers::calibration::Signal;
use saturn_protocol::ids::{ChatId, ConstraintId};
use saturn_protocol::rpc::{
    ChatNotice, ConstraintActor, ConstraintChangeInfo, ConstraintChangeKind,
    ConstraintExceptionKind, ConstraintInfo, ConstraintStatus, QueryResult, Request,
};

use crate::store::{
    Actor, ConstraintState, EventKind, ExceptionKind, NO_INPUT, NewRegistration, NewRule,
    UndoOutcome,
};
use crate::{Engine, EngineError};

impl Engine {
    /// 사용자의 제약 변경 요청을 처리기로 나눈다. 제약 변경 요청이 아니면 아무것도 하지 않는다.
    ///
    /// # Errors
    /// 각 처리기의 오류.
    pub(crate) async fn route_constraint(&mut self, request: Request) -> Result<(), EngineError> {
        match request {
            Request::ReleaseConstraint {
                constraint,
                revision,
                mistaken,
            } => {
                self.release_constraint(constraint, revision, mistaken)
                    .await
            }
            Request::AddConstraint { chat, text } => self.add_constraint(chat, &text).await,
            Request::UndoConstraintChange {
                constraint,
                event,
                revision,
            } => {
                self.undo_constraint_change(constraint, event, revision)
                    .await
            }
            _ => Ok(()),
        }
    }

    // cost: time O(n), heap O(n), stack O(1), io 1
    // vars: n = 글자 수
    // basis: estimate
    /// 사용자가 제약을 직접 등록한다. 앞뒤 공백만 떼고 원문 그대로 저장하며 의미를 판단하지 않는다. router도
    /// `constraint.auto_apply`도 권한 모드도 거치지 않고 바로 유효 제약이 되어 인계 패킷의 제약 칸에 들어간다.
    ///
    /// # Errors
    /// 빈 글이면 `EmptyConstraint`, 쓰기 실패면 `Store`.
    pub(crate) async fn add_constraint(
        &mut self,
        chat: ChatId,
        text: &str,
    ) -> Result<(), EngineError> {
        let rule = text.trim();
        if rule.is_empty() {
            return Err(EngineError::EmptyConstraint);
        }
        self.store
            .register_constraints(&NewRegistration {
                chat,
                input: NO_INPUT,
                rules: &[NewRule {
                    line: 0,
                    rule: rule.to_owned(),
                    scope: scope_of(rule),
                }],
                state: ConstraintState::Active,
                actor: Actor::User,
                reason: None,
                judgment: None,
            })
            .await?;
        self.notify_chat(
            chat,
            ChatNotice::ConstraintAdded {
                rule: rule.to_owned(),
                unconfirmed: false,
            },
        )
        .await;
        Ok(())
    }

    /// 사용자가 변경 한 건을 되돌린다. 화면이 본 제약 revision이 지금과 다르거나 가장 최근 변경이 아니면 거절한다.
    /// router가 한 변경을 되돌렸으면 그 판단의 결과 신호를 `Wrong`으로 남긴다.
    ///
    /// # Errors
    /// 낡았거나 되돌릴 수 없으면 `StaleConstraint`, 쓰기 실패면 `Store`.
    pub(crate) async fn undo_constraint_change(
        &mut self,
        constraint: ConstraintId,
        event: u64,
        revision: u64,
    ) -> Result<(), EngineError> {
        let chat = self
            .store
            .chat_of_constraint(constraint)
            .await?
            .ok_or(EngineError::StaleConstraint)?;
        let event = i64::try_from(event).map_err(|_| EngineError::StaleConstraint)?;
        let outcome = self
            .store
            .undo_constraint_change(constraint, event, revision)
            .await?;
        let UndoOutcome::Applied {
            rule,
            undone: (actor, judgment),
        } = outcome
        else {
            return Err(EngineError::StaleConstraint);
        };
        if let (Actor::Router, Some(judgment)) = (actor, judgment) {
            self.note_reaction(judgment, Signal::Wrong);
        }
        self.notify_chat(chat, ChatNotice::ConstraintRestored { rule })
            .await;
        Ok(())
    }

    /// 잘못 등록으로 해제한 제약을 등록한 판단의 결과 신호를 `Wrong`으로 남긴다. 오류는 로그만 남긴다.
    pub(crate) async fn note_wrong_registration(&mut self, constraint: ConstraintId) {
        match self.store.registration_judgment(constraint).await {
            Ok(Some(judgment)) => self.note_reaction(judgment, Signal::Wrong),
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(error = %self.failure_line(&error), "failed to read the registration judgment");
            }
        }
    }

    /// `ListConstraints`의 답. 목록과 변경 내역과 읽은 때의 제약 revision을 한 번에 돌려준다.
    ///
    /// # Errors
    /// 읽기 실패면 `Store`.
    pub(crate) async fn constraint_list_result(
        &self,
        chat: ChatId,
    ) -> Result<QueryResult, EngineError> {
        let revision = self.store.constraint_revision(chat).await?;
        let constraints = self
            .store
            .constraints_of_chat(chat)
            .await?
            .into_iter()
            .map(|stored| ConstraintInfo {
                id: stored.id,
                rule: stored.rule,
                scope: stored.scope,
                status: match stored.state {
                    ConstraintState::Candidate => ConstraintStatus::Candidate,
                    ConstraintState::Active => ConstraintStatus::Active,
                    ConstraintState::Released => ConstraintStatus::Released,
                },
                exception: stored.exception.map(|exception| {
                    let kind = match exception.kind {
                        ExceptionKind::Once => ConstraintExceptionKind::Once,
                        ExceptionKind::Scoped => ConstraintExceptionKind::Scoped,
                    };
                    (kind, exception.condition)
                }),
            })
            .collect();
        let changes = self
            .store
            .constraint_changes_of_chat(chat)
            .await?
            .into_iter()
            .map(|change| ConstraintChangeInfo {
                event: u64::try_from(change.event).unwrap_or_default(),
                constraint: change.constraint,
                rule: change.rule,
                kind: match change.kind {
                    EventKind::Added => ConstraintChangeKind::Added,
                    EventKind::Released => ConstraintChangeKind::Released,
                    EventKind::Excepted => ConstraintChangeKind::Excepted,
                    EventKind::Resumed => ConstraintChangeKind::Resumed,
                    EventKind::Restored => ConstraintChangeKind::Restored,
                },
                actor: match change.actor {
                    Actor::Router => ConstraintActor::Router,
                    Actor::User => ConstraintActor::User,
                    Actor::Engine => ConstraintActor::Engine,
                },
                undoable: change.undoable,
                at_ms: u64::try_from(change.at).unwrap_or_default(),
            })
            .collect();
        Ok(QueryResult::Constraints {
            chat,
            revision,
            constraints,
            changes,
        })
    }
}
