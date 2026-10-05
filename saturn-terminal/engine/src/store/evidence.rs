//! 실행마다 완료 검사 근거: 마지막 수정 뒤 검사가 통과했는지의 판정과 근거 이벤트 번호.
//! 설계: docs/design/records.md#실행별-완료-검사-근거

use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::RunId;
use saturn_protocol::state::CompletionEvidence;
use sqlx::Row;

use super::{Store, StoreError, enum_text, parse_enum, to_sql_int};

impl Store {
    /// 같은 실행을 다시 기록하면 바꿔 쓴다.
    ///
    /// # Errors
    /// 쓰기 실패면 `Database`.
    pub(crate) async fn record_completion(
        &self,
        run: RunId,
        evidence: &CompletionEvidence,
    ) -> Result<(), StoreError> {
        let reason = evidence.reason.as_ref().map(enum_text).transpose()?;
        let events = serde_json::to_string(&evidence.events)?;
        sqlx::query(
            "UPDATE runs SET evidence_state = ?, evidence_reason = ?, evidence_events = ? WHERE id = ?",
        )
        .bind(enum_text(&evidence.state)?)
        .bind(reason)
        .bind(events)
        .bind(to_sql_int(run.0))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 기록하지 않은 실행(이관 전, 중간에 끊긴 실행)은 `None`.
    ///
    /// # Errors
    /// 읽기 실패면 `Database`, 저장된 값이 깨졌으면 `Json`.
    pub(crate) async fn run_completion(
        &self,
        run: RunId,
    ) -> Result<Option<CompletionEvidence>, StoreError> {
        let row = sqlx::query(
            "SELECT evidence_state, evidence_reason, evidence_events FROM runs WHERE id = ?",
        )
        .bind(to_sql_int(run.0))
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        completion_of(&row)
    }

    /// 실행에 붙은 이벤트를 채팅 안의 기록 번호와 함께 기록 순서로 읽는다.
    ///
    /// # Errors
    /// 저장된 JSON이 깨졌으면 `Json`.
    pub(crate) async fn run_events_numbered(
        &self,
        run: RunId,
    ) -> Result<Vec<(u64, ProviderEvent)>, StoreError> {
        let rows = sqlx::query("SELECT seq, body FROM events WHERE run_id = ? ORDER BY seq")
            .bind(to_sql_int(run.0))
            .fetch_all(&self.pool)
            .await?;
        rows.iter()
            .map(|row| {
                let seq: i64 = row.try_get("seq")?;
                Ok((
                    u64::try_from(seq).unwrap_or_default(),
                    serde_json::from_str(row.try_get("body")?)?,
                ))
            })
            .collect()
    }
}

/// `runs` 행의 근거 열에서 판정을 읽는다. 상태 열이 비어 있으면 `None`.
fn completion_of(row: &sqlx::sqlite::SqliteRow) -> Result<Option<CompletionEvidence>, StoreError> {
    let Some(state) = row.try_get::<Option<String>, _>("evidence_state")? else {
        return Ok(None);
    };
    let reason = row
        .try_get::<Option<String>, _>("evidence_reason")?
        .map(|text| parse_enum(&text))
        .transpose()?;
    let events = row
        .try_get::<Option<String>, _>("evidence_events")?
        .map(|text| serde_json::from_str(&text))
        .transpose()?
        .unwrap_or_default();
    Ok(Some(CompletionEvidence {
        state: parse_enum(&state)?,
        reason,
        events,
    }))
}

#[cfg(test)]
mod tests {
    use saturn_protocol::state::{EvidenceState, UnverifiedReason};

    use super::*;
    use crate::store::records::tests::chat_with_run;
    use crate::store::tests::temp_store;

    #[tokio::test]
    async fn completion_round_trips_and_is_absent_until_recorded() {
        let (_dir, store) = temp_store().await;
        let (_, _, _, run) = chat_with_run(&store).await;
        assert_eq!(store.run_completion(run).await.unwrap(), None);
        let verified = CompletionEvidence {
            state: EvidenceState::Verified,
            reason: None,
            events: vec![4, 9],
        };
        let failed = CompletionEvidence {
            state: EvidenceState::Unverified,
            reason: Some(UnverifiedReason::CheckFailed),
            events: Vec::new(),
        };

        store.record_completion(run, &verified).await.unwrap();
        assert_eq!(store.run_completion(run).await.unwrap(), Some(verified));
        store.record_completion(run, &failed).await.unwrap();

        assert_eq!(store.run_completion(run).await.unwrap(), Some(failed));
    }
}
