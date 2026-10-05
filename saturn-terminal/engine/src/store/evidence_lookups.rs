//! 에이전트 작업이 기록을 찾거나 원문을 다시 읽은 시도. 조회 횟수와 돌려준 양을 세는 재료이고 원문과 검색어는 두지 않는다.
//! 설계: docs/design/records.md#근거-조회-기록

use std::time::SystemTime;

use saturn_protocol::ids::{ChatId, LedgerSeq};
use sqlx::Row;

use super::{Store, StoreError, from_sql_int, to_millis, to_sql_int};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LookupKind {
    Search,
    Read,
}

impl LookupKind {
    fn name(self) -> &'static str {
        match self {
            Self::Search => "Search",
            Self::Read => "Read",
        }
    }
}

/// 조회의 결과. 거절은 원문을 돌려주지 않았다는 뜻이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LookupOutcome {
    Ok,
    NotFound,
    Stale,
    /// 읽기 범위 밖 파일의 기록.
    Scope,
}

impl LookupOutcome {
    fn name(self) -> &'static str {
        match self {
            Self::Ok => "Ok",
            Self::NotFound => "NotFound",
            Self::Stale => "Stale",
            Self::Scope => "Scope",
        }
    }
}

/// 저장한 조회 한 건. `units`는 검색이면 돌려준 후보 수, 읽기면 돌려준 글자 수다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EvidenceLookup {
    pub(crate) kind: String,
    pub(crate) record: Option<LedgerSeq>,
    pub(crate) outcome: String,
    pub(crate) units: u64,
}

impl Store {
    /// 조회 시도를 한 행으로 쓴다.
    ///
    /// # Errors
    /// 쓰기 실패면 `Database`.
    pub(crate) async fn record_evidence_lookup(
        &self,
        chat: ChatId,
        (kind, record): (LookupKind, Option<LedgerSeq>),
        (outcome, units): (LookupOutcome, u64),
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO evidence_lookups (chat_id, kind, record_id, outcome, units, created_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(to_sql_int(chat.0))
        .bind(kind.name())
        .bind(record.map(|seq| to_sql_int(seq.0)))
        .bind(outcome.name())
        .bind(to_sql_int(units))
        .bind(to_millis(SystemTime::now()))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 채팅의 조회 시도를 일어난 순서로 읽는다.
    ///
    /// # Errors
    /// 읽기 실패면 `Database`.
    pub(crate) async fn evidence_lookups(
        &self,
        chat: ChatId,
    ) -> Result<Vec<EvidenceLookup>, StoreError> {
        let rows = sqlx::query(
            "SELECT kind, record_id, outcome, units FROM evidence_lookups WHERE chat_id = ? ORDER BY id",
        )
        .bind(to_sql_int(chat.0))
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(EvidenceLookup {
                    kind: row.try_get("kind")?,
                    record: row
                        .try_get::<Option<i64>, _>("record_id")?
                        .map(|id| LedgerSeq(from_sql_int(id))),
                    outcome: row.try_get("outcome")?,
                    units: from_sql_int(row.try_get("units")?),
                })
            })
            .collect()
    }
}
