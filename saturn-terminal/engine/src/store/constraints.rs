//! 제약 표: 규칙 한 줄과 범위, 변경 이벤트, 사용자에게 묻는 확인. 모든 변경은 상태와 이벤트를 한 거래로 쓴다.
//! 설계: docs/design/records.md, docs/design/constraints.md

use std::time::SystemTime;

use saturn_protocol::ids::{ChatId, ConstraintAskId, ConstraintId, InputId, JudgmentId, SessionId};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use super::{Store, StoreError, enum_text, from_sql_int, parse_enum, to_millis, to_sql_int};

/// 상태를 바꾸는 이벤트가 이 표의 상태와 늘 같이 쓰인다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ConstraintState {
    /// 등록할지 사용자에게 묻는 중이다.
    Candidate,
    Active,
    /// 사용자나 판단이 해제했거나 등록을 거절했다.
    Released,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum EventKind {
    Added,
    Released,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Actor {
    Router,
    User,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum EventReason {
    /// 등록을 사용자에게 물어 거절했다. 대화 기록에는 줄을 남기지 않는다.
    Declined,
    /// 그 입력이 취소돼 제약을 함께 해제했다.
    InputCanceled,
    /// 권한 모드 `full`이라 묻지 않고 정한 변경이다.
    Unconfirmed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum AskKind {
    Register,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum AskAnswerText {
    Yes,
    No,
    /// 대상 제약이 바뀌어 닫았다.
    Void,
}

/// 입력에서 자른 규칙 한 줄. `line`은 나누지 않았으면 0, 나눴으면 문장 번호(1부터)다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NewRule {
    pub(crate) line: u32,
    pub(crate) rule: String,
    /// 비어 있으면 적용 범위가 전체다.
    pub(crate) scope: Vec<String>,
}

/// 한 입력에서 나온 규칙을 한 거래로 쓴다.
#[derive(Debug, Clone)]
pub(crate) struct NewRegistration<'a> {
    pub(crate) chat: ChatId,
    pub(crate) input: InputId,
    pub(crate) rules: &'a [NewRule],
    /// `Active`면 등록 이벤트를 쓰고, `Candidate`면 사용자에게 묻는 확인을 만든다.
    pub(crate) state: ConstraintState,
    pub(crate) actor: Actor,
    pub(crate) reason: Option<EventReason>,
    pub(crate) judgment: Option<JudgmentId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Registered {
    pub(crate) constraints: Vec<ConstraintId>,
    /// `Candidate`로 썼으면 만든 확인.
    pub(crate) ask: Option<ConstraintAskId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredConstraint {
    pub(crate) id: ConstraintId,
    pub(crate) chat: ChatId,
    pub(crate) input: InputId,
    pub(crate) line: u32,
    pub(crate) rule: String,
    pub(crate) scope: Vec<String>,
    pub(crate) state: ConstraintState,
}

/// 답을 기다리는 등록 확인. 규칙은 그 입력의 `Candidate` 전체다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredAsk {
    pub(crate) id: ConstraintAskId,
    pub(crate) chat: ChatId,
    pub(crate) judgment: Option<JudgmentId>,
    pub(crate) rules: Vec<String>,
}

/// 확인의 답을 적용한 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AnswerOutcome {
    /// 이미 답했거나 대상이 바뀌어 닫힌 확인이다.
    Closed,
    Applied {
        chat: ChatId,
        judgment: Option<JudgmentId>,
        /// 등록했으면 참이고 거절했으면 거짓이다.
        registered: bool,
        rules: Vec<String>,
    },
}

/// 입력이 취소돼 해제한 제약의 규칙.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CanceledConstraints {
    pub(crate) chat: ChatId,
    pub(crate) rules: Vec<String>,
    /// 함께 닫은 등록 확인. TUI의 창을 지워야 한다.
    pub(crate) closed_asks: Vec<ConstraintAskId>,
}

impl Store {
    // cost: time O(r), heap O(r), stack O(1), io r
    // vars: r = 규칙 수
    // basis: estimate
    /// 규칙마다 제약 행을 쓰고, `Active`면 `Added` 이벤트를, `Candidate`면 확인 하나를 같은 거래에 쓴다.
    ///
    /// # Errors
    /// 쓰기 실패면 `Database`이고 아무것도 쓰지 않는다.
    pub(crate) async fn register_constraints(
        &self,
        new: &NewRegistration<'_>,
    ) -> Result<Registered, StoreError> {
        let now = to_millis(SystemTime::now());
        let mut tx = self.pool.begin().await?;
        let mut ids = Vec::with_capacity(new.rules.len());
        for rule in new.rules {
            let scope = (!rule.scope.is_empty()).then(|| rule.scope.join("\n"));
            let id: i64 = sqlx::query_scalar(
                "INSERT INTO constraints (chat_id, input_id, line, rule, scope, state, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?) RETURNING constraint_id",
            )
            .bind(to_sql_int(new.chat.0))
            .bind(to_sql_int(new.input.0))
            .bind(i64::from(rule.line))
            .bind(&rule.rule)
            .bind(scope)
            .bind(enum_text(&new.state)?)
            .bind(now)
            .fetch_one(&mut *tx)
            .await?;
            let constraint = ConstraintId(from_sql_int(id));
            if new.state == ConstraintState::Active {
                let added = NewEvent {
                    chat: new.chat,
                    constraint,
                    kind: EventKind::Added,
                    actor: new.actor,
                    reason: new.reason,
                    input: new.input,
                    judgment: new.judgment,
                    at: now,
                };
                insert_event(&mut tx, &added).await?;
            }
            ids.push(constraint);
        }
        let ask = match (new.state, ids.first()) {
            (ConstraintState::Candidate, Some(first)) => {
                let id: i64 = sqlx::query_scalar(
                    "INSERT INTO constraint_asks (chat_id, kind, constraint_id, judgment_id, asked_at) \
                     VALUES (?, ?, ?, ?, ?) RETURNING ask_id",
                )
                .bind(to_sql_int(new.chat.0))
                .bind(enum_text(&AskKind::Register)?)
                .bind(to_sql_int(first.0))
                .bind(new.judgment.map(|judgment| to_sql_int(judgment.0)))
                .bind(now)
                .fetch_one(&mut *tx)
                .await?;
                Some(ConstraintAskId(from_sql_int(id)))
            }
            _ => None,
        };
        tx.commit().await?;
        Ok(Registered {
            constraints: ids,
            ask,
        })
    }

    // cost: time O(c), heap O(1), stack O(1), io c
    // vars: c = 기록할 제약 수
    // basis: estimate
    /// 전환에서 새 session의 패킷에 든 제약별 단계를 한 거래로 쓴다. 같은 session의 같은 제약은 한 행이다.
    ///
    /// # Errors
    /// 쓰기 실패면 `Database`.
    pub(crate) async fn record_packet_constraints(
        &self,
        session: SessionId,
        tiers: &[(ConstraintId, &str)],
    ) -> Result<(), StoreError> {
        let mut transaction = self.pool.begin().await?;
        for (constraint, tier) in tiers {
            sqlx::query(
                "INSERT OR REPLACE INTO packet_constraints (session_id, constraint_id, tier) \
                 VALUES (?, ?, ?)",
            )
            .bind(to_sql_int(session.0))
            .bind(to_sql_int(constraint.0))
            .bind(*tier)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    /// 새 session의 패킷 제약 기록. 제약 번호 순서다.
    ///
    /// # Errors
    /// 읽기 실패면 `Database`.
    #[cfg(test)]
    pub(crate) async fn packet_constraints_of(
        &self,
        session: SessionId,
    ) -> Result<Vec<(ConstraintId, String)>, StoreError> {
        let rows = sqlx::query(
            "SELECT constraint_id, tier FROM packet_constraints WHERE session_id = ? \
             ORDER BY constraint_id",
        )
        .bind(to_sql_int(session.0))
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok((
                    ConstraintId(from_sql_int(row.try_get("constraint_id")?)),
                    row.try_get("tier")?,
                ))
            })
            .collect()
    }

    /// 채팅의 제약 revision. 마지막 `constraint_events` 번호이고 이벤트가 없으면 0이다.
    ///
    /// # Errors
    /// 읽기 실패면 `Database`.
    #[cfg(test)]
    pub(crate) async fn constraint_revision(&self, chat: ChatId) -> Result<u64, StoreError> {
        let revision: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(event_id), 0) FROM constraint_events WHERE chat_id = ?",
        )
        .bind(to_sql_int(chat.0))
        .fetch_one(&self.pool)
        .await?;
        Ok(from_sql_int(revision))
    }

    /// 채팅의 제약을 만든 순서로.
    ///
    /// # Errors
    /// 읽기 실패면 `Database`, 저장된 상태가 깨졌으면 `Json`.
    pub(crate) async fn constraints_of_chat(
        &self,
        chat: ChatId,
    ) -> Result<Vec<StoredConstraint>, StoreError> {
        let rows = sqlx::query(
            "SELECT constraint_id, chat_id, input_id, line, rule, scope, state FROM constraints \
             WHERE chat_id = ? ORDER BY constraint_id",
        )
        .bind(to_sql_int(chat.0))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(constraint_from_row).collect()
    }

    /// 입력에서 나온 제약을 만든 순서로.
    ///
    /// # Errors
    /// `constraints_of_chat`과 같다.
    pub(crate) async fn constraints_of_input(
        &self,
        input: InputId,
    ) -> Result<Vec<StoredConstraint>, StoreError> {
        let rows = sqlx::query(
            "SELECT constraint_id, chat_id, input_id, line, rule, scope, state FROM constraints \
             WHERE input_id = ? ORDER BY constraint_id",
        )
        .bind(to_sql_int(input.0))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(constraint_from_row).collect()
    }

    /// 답을 기다리는 등록 확인을 물은 순서로. engine을 다시 켤 때 TUI에 되살린다.
    ///
    /// # Errors
    /// `constraints_of_chat`과 같다.
    pub(crate) async fn open_constraint_asks(&self) -> Result<Vec<StoredAsk>, StoreError> {
        let rows = sqlx::query(
            "SELECT ask_id, chat_id, judgment_id, constraint_id FROM constraint_asks \
             WHERE answer IS NULL ORDER BY ask_id",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut asks = Vec::with_capacity(rows.len());
        for row in &rows {
            let first: i64 = row.try_get("constraint_id")?;
            let input: i64 =
                sqlx::query_scalar("SELECT input_id FROM constraints WHERE constraint_id = ?")
                    .bind(first)
                    .fetch_one(&self.pool)
                    .await?;
            let rules = self
                .constraints_of_input(InputId(from_sql_int(input)))
                .await?
                .into_iter()
                .filter(|constraint| constraint.state == ConstraintState::Candidate)
                .map(|constraint| constraint.rule)
                .collect();
            asks.push(StoredAsk {
                id: ConstraintAskId(from_sql_int(row.try_get("ask_id")?)),
                chat: ChatId(from_sql_int(row.try_get("chat_id")?)),
                judgment: row
                    .try_get::<Option<i64>, _>("judgment_id")?
                    .map(|id| JudgmentId(from_sql_int(id))),
                rules,
            });
        }
        Ok(asks)
    }

    // cost: time O(r), heap O(r), stack O(1), io r
    // vars: r = 그 입력의 후보 제약 수
    // basis: estimate
    /// 사용자의 답을 한 거래로 적용한다. `Yes`는 그 입력의 후보 제약 전체를 `Active`로 바꾸고 `Added` 이벤트를 쓴다.
    /// `No`는 `Released`(`Declined`)로 바꾼다. 이미 답했거나 대상 제약이 `Candidate`가 아니면 닫힌 확인이라
    /// `Closed`이고 늦은 답은 적용하지 않는다.
    ///
    /// # Errors
    /// 없는 확인이면 `NotFound`, 쓰기 실패면 `Database`이고 아무것도 쓰지 않는다.
    pub(crate) async fn answer_constraint_ask(
        &self,
        ask: ConstraintAskId,
        register: bool,
    ) -> Result<AnswerOutcome, StoreError> {
        let now = to_millis(SystemTime::now());
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT chat_id, judgment_id, constraint_id, answer FROM constraint_asks WHERE ask_id = ?",
        )
        .bind(to_sql_int(ask.0))
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| StoreError::NotFound {
            what: format!("constraint ask {}", ask.0),
        })?;
        if row.try_get::<Option<String>, _>("answer")?.is_some() {
            return Ok(AnswerOutcome::Closed);
        }
        let chat = ChatId(from_sql_int(row.try_get("chat_id")?));
        let judgment = row
            .try_get::<Option<i64>, _>("judgment_id")?
            .map(|id| JudgmentId(from_sql_int(id)));
        let first: i64 = row.try_get("constraint_id")?;
        let input: i64 =
            sqlx::query_scalar("SELECT input_id FROM constraints WHERE constraint_id = ?")
                .bind(first)
                .fetch_one(&mut *tx)
                .await?;
        let rows = sqlx::query(
            "SELECT constraint_id, chat_id, input_id, line, rule, scope, state FROM constraints \
             WHERE input_id = ? AND state = ? ORDER BY constraint_id",
        )
        .bind(input)
        .bind(enum_text(&ConstraintState::Candidate)?)
        .fetch_all(&mut *tx)
        .await?;
        let candidates = rows
            .iter()
            .map(constraint_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        let answer = if candidates.is_empty() {
            AskAnswerText::Void
        } else if register {
            AskAnswerText::Yes
        } else {
            AskAnswerText::No
        };
        sqlx::query("UPDATE constraint_asks SET answer = ?, answered_at = ? WHERE ask_id = ?")
            .bind(enum_text(&answer)?)
            .bind(now)
            .bind(to_sql_int(ask.0))
            .execute(&mut *tx)
            .await?;
        if answer == AskAnswerText::Void {
            tx.commit().await?;
            return Ok(AnswerOutcome::Closed);
        }
        let (state, kind, reason) = if register {
            (ConstraintState::Active, EventKind::Added, None)
        } else {
            (
                ConstraintState::Released,
                EventKind::Released,
                Some(EventReason::Declined),
            )
        };
        for candidate in &candidates {
            sqlx::query("UPDATE constraints SET state = ? WHERE constraint_id = ?")
                .bind(enum_text(&state)?)
                .bind(to_sql_int(candidate.id.0))
                .execute(&mut *tx)
                .await?;
            let event = NewEvent {
                chat,
                constraint: candidate.id,
                kind,
                actor: Actor::User,
                reason,
                input: candidate.input,
                judgment,
                at: now,
            };
            insert_event(&mut tx, &event).await?;
        }
        tx.commit().await?;
        Ok(AnswerOutcome::Applied {
            chat,
            judgment,
            registered: register,
            rules: candidates
                .into_iter()
                .map(|constraint| constraint.rule)
                .collect(),
        })
    }

    // cost: time O(r), heap O(r), stack O(1), io r
    // vars: r = 그 입력의 제약 수
    // basis: estimate
    /// 입력이 취소되면 그 입력의 제약을 한 거래로 `Released`(`InputCanceled`)로 바꾸고 열린 확인을 닫는다.
    /// 해제한 제약이 없으면 `None`이다. 사용자가 거둔 말이 제약으로 남지 않게 하기 위해서다.
    ///
    /// # Errors
    /// 쓰기 실패면 `Database`이고 아무것도 쓰지 않는다.
    pub(crate) async fn release_input_constraints(
        &self,
        input: InputId,
    ) -> Result<Option<CanceledConstraints>, StoreError> {
        let now = to_millis(SystemTime::now());
        let mut tx = self.pool.begin().await?;
        let rows = sqlx::query(
            "SELECT constraint_id, chat_id, input_id, line, rule, scope, state FROM constraints \
             WHERE input_id = ? AND state != ? ORDER BY constraint_id",
        )
        .bind(to_sql_int(input.0))
        .bind(enum_text(&ConstraintState::Released)?)
        .fetch_all(&mut *tx)
        .await?;
        let live = rows
            .iter()
            .map(constraint_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        let Some(first) = live.first() else {
            return Ok(None);
        };
        let chat = first.chat;
        let mut closed_asks = Vec::new();
        for constraint in &live {
            sqlx::query("UPDATE constraints SET state = ? WHERE constraint_id = ?")
                .bind(enum_text(&ConstraintState::Released)?)
                .bind(to_sql_int(constraint.id.0))
                .execute(&mut *tx)
                .await?;
            let open: Vec<i64> = sqlx::query_scalar(
                "UPDATE constraint_asks SET answer = ?, answered_at = ? \
                 WHERE constraint_id = ? AND answer IS NULL RETURNING ask_id",
            )
            .bind(enum_text(&AskAnswerText::Void)?)
            .bind(now)
            .bind(to_sql_int(constraint.id.0))
            .fetch_all(&mut *tx)
            .await?;
            closed_asks.extend(open.into_iter().map(|id| ConstraintAskId(from_sql_int(id))));
            let event = NewEvent {
                chat,
                constraint: constraint.id,
                kind: EventKind::Released,
                actor: Actor::User,
                reason: Some(EventReason::InputCanceled),
                input,
                judgment: None,
                at: now,
            };
            insert_event(&mut tx, &event).await?;
        }
        tx.commit().await?;
        Ok(Some(CanceledConstraints {
            chat,
            rules: live.into_iter().map(|constraint| constraint.rule).collect(),
            closed_asks,
        }))
    }

    /// 변경 이벤트를 쓴 순서로 `(종류, 주체, 사유, 판단 기록)`. 시험이 이벤트 행을 확인하는 데 쓴다.
    #[cfg(test)]
    pub(crate) async fn constraint_events_of_chat(
        &self,
        chat: ChatId,
    ) -> Result<Vec<(EventKind, Actor, Option<EventReason>, Option<JudgmentId>)>, StoreError> {
        let rows = sqlx::query(
            "SELECT kind, actor, reason, judgment_id FROM constraint_events \
             WHERE chat_id = ? ORDER BY event_id",
        )
        .bind(to_sql_int(chat.0))
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok((
                    parse_enum(&row.try_get::<String, _>("kind")?)?,
                    parse_enum(&row.try_get::<String, _>("actor")?)?,
                    row.try_get::<Option<String>, _>("reason")?
                        .map(|text| parse_enum(&text))
                        .transpose()?,
                    row.try_get::<Option<i64>, _>("judgment_id")?
                        .map(|id| JudgmentId(from_sql_int(id))),
                ))
            })
            .collect()
    }
}

/// 이벤트 한 줄. 행은 이어 쓰기만 한다.
struct NewEvent {
    chat: ChatId,
    constraint: ConstraintId,
    kind: EventKind,
    actor: Actor,
    reason: Option<EventReason>,
    input: InputId,
    judgment: Option<JudgmentId>,
    at: i64,
}

async fn insert_event(tx: &mut sqlx::SqliteConnection, event: &NewEvent) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO constraint_events (chat_id, constraint_id, kind, actor, reason, input_id, judgment_id, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(to_sql_int(event.chat.0))
    .bind(to_sql_int(event.constraint.0))
    .bind(enum_text(&event.kind)?)
    .bind(enum_text(&event.actor)?)
    .bind(event.reason.map(|reason| enum_text(&reason)).transpose()?)
    .bind(to_sql_int(event.input.0))
    .bind(event.judgment.map(|judgment| to_sql_int(judgment.0)))
    .bind(event.at)
    .execute(tx)
    .await?;
    Ok(())
}

fn constraint_from_row(row: &SqliteRow) -> Result<StoredConstraint, StoreError> {
    let scope: Option<String> = row.try_get("scope")?;
    Ok(StoredConstraint {
        id: ConstraintId(from_sql_int(row.try_get("constraint_id")?)),
        chat: ChatId(from_sql_int(row.try_get("chat_id")?)),
        input: InputId(from_sql_int(row.try_get("input_id")?)),
        line: u32::try_from(row.try_get::<i64, _>("line")?).unwrap_or_default(),
        rule: row.try_get("rule")?,
        scope: scope
            .map(|text| text.lines().map(str::to_owned).collect())
            .unwrap_or_default(),
        state: parse_enum(&row.try_get::<String, _>("state")?)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::HistoryEntry;
    use crate::store::records::tests::new_input;
    use crate::store::tests::temp_store;

    fn rules(texts: &[&str]) -> Vec<NewRule> {
        texts
            .iter()
            .enumerate()
            .map(|(index, text)| NewRule {
                line: u32::try_from(index).unwrap(),
                rule: (*text).to_owned(),
                scope: Vec::new(),
            })
            .collect()
    }

    async fn chat_and_input(store: &Store) -> (ChatId, InputId) {
        let chat = store.create_chat("/work".into()).await.unwrap();
        let input = store.accept_input(&new_input(chat, "text")).await.unwrap();
        (chat, input)
    }

    fn registration<'a>(
        chat: ChatId,
        input: InputId,
        rules: &'a [NewRule],
        state: ConstraintState,
    ) -> NewRegistration<'a> {
        NewRegistration {
            chat,
            input,
            rules,
            state,
            actor: Actor::Router,
            reason: None,
            judgment: None,
        }
    }

    #[tokio::test]
    async fn constraint_active_registration_writes_rows_and_events_in_one_transaction() {
        let (_dir, store) = temp_store().await;
        let (chat, input) = chat_and_input(&store).await;
        let new_rules = vec![NewRule {
            line: 0,
            rule: "src/auth/ 에서 unwrap 금지".to_owned(),
            scope: vec!["src/auth/".to_owned(), "src/lib.rs".to_owned()],
        }];

        let registered = store
            .register_constraints(&registration(
                chat,
                input,
                &new_rules,
                ConstraintState::Active,
            ))
            .await
            .unwrap();

        assert_eq!(registered.constraints.len(), 1);
        assert_eq!(registered.ask, None);
        let stored = store.constraints_of_chat(chat).await.unwrap();
        assert_eq!(
            stored[0].scope,
            vec!["src/auth/".to_owned(), "src/lib.rs".to_owned()]
        );
        assert_eq!(stored[0].state, ConstraintState::Active);
        assert_eq!(store.constraint_revision(chat).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn constraint_revision_is_the_last_event_number_and_zero_without_events() {
        let (_dir, store) = temp_store().await;
        let (chat, input) = chat_and_input(&store).await;
        let other = store.create_chat("/other".into()).await.unwrap();
        let other_input = store.accept_input(&new_input(other, "text")).await.unwrap();
        assert_eq!(store.constraint_revision(chat).await.unwrap(), 0);

        let two = rules(&["a", "b"]);
        store
            .register_constraints(&registration(chat, input, &two, ConstraintState::Active))
            .await
            .unwrap();
        let one = rules(&["c"]);
        store
            .register_constraints(&registration(
                other,
                other_input,
                &one,
                ConstraintState::Active,
            ))
            .await
            .unwrap();

        assert_eq!(store.constraint_revision(chat).await.unwrap(), 2);
        assert_eq!(store.constraint_revision(other).await.unwrap(), 3);
    }

    #[tokio::test]
    async fn constraint_registration_that_fails_midway_writes_nothing() {
        let (_dir, store) = temp_store().await;
        let (chat, input) = chat_and_input(&store).await;
        sqlx::raw_sql("DROP TABLE constraint_events")
            .execute(&store.pool)
            .await
            .unwrap();
        let new_rules = rules(&["a"]);

        let failed = store
            .register_constraints(&registration(
                chat,
                input,
                &new_rules,
                ConstraintState::Active,
            ))
            .await;

        assert!(failed.is_err());
        assert!(store.constraints_of_chat(chat).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn constraint_candidate_gets_one_ask_and_answers_apply_to_the_whole_input() {
        let (_dir, store) = temp_store().await;
        let (chat, input) = chat_and_input(&store).await;
        let two = rules(&["first rule", "second rule"]);
        let registered = store
            .register_constraints(&registration(chat, input, &two, ConstraintState::Candidate))
            .await
            .unwrap();
        let ask = registered.ask.unwrap();

        let open = store.open_constraint_asks().await.unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(
            open[0].rules,
            vec!["first rule".to_owned(), "second rule".to_owned()]
        );
        assert!(
            store
                .constraint_events_of_chat(chat)
                .await
                .unwrap()
                .is_empty()
        );

        let outcome = store.answer_constraint_ask(ask, true).await.unwrap();

        assert!(matches!(
            outcome,
            AnswerOutcome::Applied { registered: true, ref rules, .. } if rules.len() == 2
        ));
        let states: Vec<ConstraintState> = store
            .constraints_of_chat(chat)
            .await
            .unwrap()
            .iter()
            .map(|constraint| constraint.state)
            .collect();
        assert_eq!(states, vec![ConstraintState::Active; 2]);
        assert_eq!(
            store.constraint_events_of_chat(chat).await.unwrap().len(),
            2
        );
        assert!(store.open_constraint_asks().await.unwrap().is_empty());
        assert_eq!(
            store.answer_constraint_ask(ask, false).await.unwrap(),
            AnswerOutcome::Closed
        );
    }

    #[tokio::test]
    async fn constraint_ask_for_an_unknown_id_is_not_found() {
        let (_dir, store) = temp_store().await;

        let missing = store.answer_constraint_ask(ConstraintAskId(9), true).await;

        assert!(matches!(missing, Err(StoreError::NotFound { .. })));
    }

    #[tokio::test]
    async fn constraint_open_asks_survive_reopening_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = Store::open(dir.path()).await.unwrap();
        let (chat, input) = chat_and_input(&store).await;
        let one = rules(&["keep it short"]);
        store
            .register_constraints(&registration(chat, input, &one, ConstraintState::Candidate))
            .await
            .unwrap();
        store.pool.close().await;

        let (store, _) = Store::open(dir.path()).await.unwrap();

        let open = store.open_constraint_asks().await.unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].chat, chat);
        assert_eq!(open[0].rules, vec!["keep it short".to_owned()]);
    }

    #[tokio::test]
    async fn constraint_history_shows_added_and_released_lines_but_not_declined() {
        let (_dir, store) = temp_store().await;
        let (chat, input) = chat_and_input(&store).await;
        let one = rules(&["answer in English"]);
        store
            .register_constraints(&registration(chat, input, &one, ConstraintState::Active))
            .await
            .unwrap();
        let declined_input = store
            .accept_input(&new_input(chat, "text 2"))
            .await
            .unwrap();
        let other = rules(&["use tabs"]);
        let registered = store
            .register_constraints(&registration(
                chat,
                declined_input,
                &other,
                ConstraintState::Candidate,
            ))
            .await
            .unwrap();
        store
            .answer_constraint_ask(registered.ask.unwrap(), false)
            .await
            .unwrap();
        store.release_input_constraints(input).await.unwrap();

        let entries = store.history_page(chat, None, 20).await.unwrap().entries;

        let lines: Vec<(EventKind, Option<EventReason>, &str)> = entries
            .iter()
            .filter_map(|entry| match entry {
                HistoryEntry::Constraint { kind, reason, rule } => {
                    Some((*kind, *reason, rule.as_str()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            lines,
            vec![
                (EventKind::Added, None, "answer in English"),
                (
                    EventKind::Released,
                    Some(EventReason::InputCanceled),
                    "answer in English"
                ),
            ]
        );
    }

    #[tokio::test]
    async fn constraint_release_for_an_input_without_constraints_does_nothing() {
        let (_dir, store) = temp_store().await;
        let (chat, input) = chat_and_input(&store).await;

        assert_eq!(store.release_input_constraints(input).await.unwrap(), None);
        assert_eq!(store.constraint_revision(chat).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn constraint_rows_are_removed_only_with_their_chat() {
        let (_dir, store) = temp_store().await;
        let (chat, input) = chat_and_input(&store).await;
        let one = rules(&["answer in English"]);
        store
            .register_constraints(&registration(chat, input, &one, ConstraintState::Candidate))
            .await
            .unwrap();

        sqlx::query("DELETE FROM chats WHERE id = ?")
            .bind(to_sql_int(chat.0))
            .execute(&store.pool)
            .await
            .unwrap();

        for table in ["constraints", "constraint_events", "constraint_asks"] {
            let rows: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
                .fetch_one(&store.pool)
                .await
                .unwrap();
            assert_eq!(rows, 0, "{table}");
        }
    }
}
