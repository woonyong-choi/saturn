//! provider에 보낸 인계 패킷의 시도별 근거. 본문은 해시와 크기만 두고 원문 항목은 번호로 가리켜 기록 원문을 복제하지 않는다.
//! 설계: docs/design/records.md#전달-패킷-근거

use std::time::SystemTime;

use saturn_protocol::ids::{ChatId, InputId, Provider, RunId, SessionId};
use sqlx::Row;

use super::{
    Store, StoreError, enum_text, from_sql_int, parse_enum, sha256_hex, to_millis, to_sql_int,
};

/// 전달 패킷 시도 하나의 번호.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PacketId(pub u64);

/// 패킷을 보낸 경우. 같은 채팅 기록에서 만든 패킷이라도 경우마다 근거가 다르다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum PacketKind {
    /// provider를 바꾸거나 이어 갈 메인이 없어 새 session의 첫 턴으로 보낸 패킷.
    Switch,
    /// 보관 session으로 돌아가며 받지 못한 변경분만 보낸 패킷.
    Return,
    /// 맥락 정리로 같은 provider의 새 session에 보낸 패킷.
    Restart,
}

/// 보냈는지의 확정 상태. 확정 미전달과 결과 불명은 다른 값이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum PacketState {
    /// provider를 부르기 직전에 기록했고 결과를 아직 모른다.
    Prepared,
    Sent,
    /// 보내지 않음이 확정이다(거절, 맥락 한도 초과).
    NotSent,
    /// 보냈는지 모른다. 다시 보내지 않는다.
    Unknown,
}

/// 패킷에 들어갔거나 빠진 재료 항목 하나. `zone`이 `Constraints`면 `ref_id`는 제약 번호이고, 대화 본문의 구역(`User`, `Steer`, `Assistant`)이면
/// `Steer`는 입력 번호, 나머지는 기록 번호이며, 그 밖은 기록 번호다.
/// 경쟁 구역 행의 `selector`는 실제 적용한 방식이다. 요청한 방식과 대체 사유는 시도 행의 `PacketSelection`에 둔다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PacketItemRow {
    pub(crate) zone: String,
    pub(crate) ref_id: u64,
    /// 보호한 대화 본문 원문의 SHA-256. 본문 항목만 갖는다.
    pub(crate) hash: Option<String>,
    pub(crate) selector: &'static str,
    /// 들어간 모양. 빠졌으면 `None`.
    pub(crate) form: Option<String>,
    /// 빠진 이유. 들어갔으면 `None`.
    pub(crate) reason: Option<&'static str>,
}

/// 시도의 경쟁 구역(도구 기록) 선별 근거. 이관 전 행과 선별을 기록하지 않는 시도는 전부 NULL이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PacketSelection {
    /// 설정이 요청한 방식(`rank`, `compact`).
    pub(crate) requested: String,
    /// 실제로 적용한 방식.
    pub(crate) actual: String,
    /// 대체했거나 일부만 답했을 때의 고정 사유.
    pub(crate) fallback: Option<String>,
    /// 질문에 보낸 후보 글에서 잘린 바이트. 판단을 요청하지 않았으면 NULL이다.
    pub(crate) omitted_bytes: Option<u64>,
    /// 질문을 만들지 못했을 때 한도를 넘은 바이트. 판단을 요청하지 않았으면 NULL이다.
    pub(crate) overflow_bytes: Option<u64>,
}

/// 보내기 직전의 패킷 한 시도.
#[derive(Debug, Clone)]
pub(crate) struct NewPacket {
    pub(crate) chat: ChatId,
    pub(crate) kind: PacketKind,
    /// 첫 시도가 1이고 맥락 한도로 거절돼 줄여 다시 보낼 때마다 1 늘어난다.
    pub(crate) attempt: u32,
    pub(crate) reduced_from: Option<PacketId>,
    /// 패킷을 받는 session.
    pub(crate) session: SessionId,
    /// 패킷을 보내게 한 입력. 맥락 정리처럼 입력 없이 보내면 `None`.
    pub(crate) input: Option<InputId>,
    pub(crate) provider: Provider,
    pub(crate) settings: u64,
    /// 패킷이 담은 기록의 마지막 번호.
    pub(crate) chat_revision: u64,
    pub(crate) constraint_revision: u64,
    pub(crate) policy: String,
    /// 보내는 글 그대로.
    pub(crate) body: String,
    pub(crate) estimated_tokens: u64,
    pub(crate) items: Vec<PacketItemRow>,
    /// 선별 근거. 기록하지 않으면 `None`이다.
    pub(crate) selection: Option<PacketSelection>,
}

/// 저장한 시도 하나. 항목은 따로 읽는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredPacket {
    pub(crate) id: PacketId,
    pub(crate) kind: PacketKind,
    pub(crate) attempt: u32,
    pub(crate) reduced_from: Option<PacketId>,
    pub(crate) session: SessionId,
    pub(crate) input: Option<InputId>,
    pub(crate) run: Option<RunId>,
    pub(crate) provider: Provider,
    pub(crate) provider_session: Option<String>,
    pub(crate) body_hash: String,
    pub(crate) body_bytes: u64,
    pub(crate) state: PacketState,
    pub(crate) selection: Option<PacketSelection>,
}

/// 시도 행의 선별 열. 요청한 방식과 실제 방식이 NULL이면 선별을 기록하지 않은 행(이관 전)이라 `None`이다.
fn selection_of(row: &sqlx::sqlite::SqliteRow) -> Result<Option<PacketSelection>, StoreError> {
    let requested: Option<String> = row.try_get("requested_selector")?;
    let actual: Option<String> = row.try_get("actual_selector")?;
    let (Some(requested), Some(actual)) = (requested, actual) else {
        return Ok(None);
    };
    Ok(Some(PacketSelection {
        requested,
        actual,
        fallback: row.try_get("selection_fallback")?,
        omitted_bytes: row
            .try_get::<Option<i64>, _>("candidate_omitted_bytes")?
            .map(from_sql_int),
        overflow_bytes: row
            .try_get::<Option<i64>, _>("state_overflow_bytes")?
            .map(from_sql_int),
    }))
}

impl Store {
    // cost: time O(i), heap O(1), stack O(1), io i
    // vars: i = 패킷 항목 수
    // basis: estimate
    /// 시도와 항목을 한 거래로 쓴다. 본문은 해시와 바이트 수만 남긴다.
    ///
    /// # Errors
    /// 쓰기 실패면 `Database`.
    pub(crate) async fn record_packet(&self, packet: &NewPacket) -> Result<PacketId, StoreError> {
        let mut tx = self.pool.begin().await?;
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO handoff_packets (chat_id, kind, attempt, reduced_from, session_id, input_id, \
             provider, settings_revision, chat_revision, constraint_revision, policy, body_hash, \
             body_bytes, estimated_tokens, state, created_at, requested_selector, actual_selector, \
             selection_fallback, candidate_omitted_bytes, state_overflow_bytes) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'Prepared', ?, ?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(to_sql_int(packet.chat.0))
        .bind(enum_text(&packet.kind)?)
        .bind(i64::from(packet.attempt))
        .bind(packet.reduced_from.map(|id| to_sql_int(id.0)))
        .bind(to_sql_int(packet.session.0))
        .bind(packet.input.map(|input| to_sql_int(input.0)))
        .bind(enum_text(&packet.provider)?)
        .bind(to_sql_int(packet.settings))
        .bind(to_sql_int(packet.chat_revision))
        .bind(to_sql_int(packet.constraint_revision))
        .bind(&packet.policy)
        .bind(sha256_hex(packet.body.as_bytes()))
        .bind(to_sql_int(packet.body.len() as u64))
        .bind(to_sql_int(packet.estimated_tokens))
        .bind(to_millis(SystemTime::now()))
        .bind(packet.selection.as_ref().map(|s| s.requested.as_str()))
        .bind(packet.selection.as_ref().map(|s| s.actual.as_str()))
        .bind(
            packet
                .selection
                .as_ref()
                .and_then(|s| s.fallback.as_deref()),
        )
        .bind(
            packet
                .selection
                .as_ref()
                .and_then(|s| s.omitted_bytes)
                .map(to_sql_int),
        )
        .bind(
            packet
                .selection
                .as_ref()
                .and_then(|s| s.overflow_bytes)
                .map(to_sql_int),
        )
        .fetch_one(&mut *tx)
        .await?;
        for item in &packet.items {
            sqlx::query(
                "INSERT INTO handoff_packet_items (packet_id, zone, ref_id, selector, form, reason, body_hash) \
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(id)
            .bind(&item.zone)
            .bind(to_sql_int(item.ref_id))
            .bind(item.selector)
            .bind(&item.form)
            .bind(item.reason)
            .bind(&item.hash)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(PacketId(from_sql_int(id)))
    }

    /// 결과를 알았을 때 상태를 확정한다. 이미 확정한 시도는 바꾸지 않는다.
    ///
    /// # Errors
    /// 쓰기 실패면 `Database`.
    pub(crate) async fn settle_packet(
        &self,
        id: PacketId,
        state: PacketState,
        provider_session: Option<&str>,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE handoff_packets SET state = ?, provider_session = COALESCE(?, provider_session) \
             WHERE id = ? AND state = 'Prepared'",
        )
        .bind(enum_text(&state)?)
        .bind(provider_session)
        .bind(to_sql_int(id.0))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 입력이 실행을 시작했을 때 그 입력을 위해 보낸 패킷에 실행 번호를 붙인다.
    ///
    /// # Errors
    /// 쓰기 실패면 `Database`.
    pub(crate) async fn link_packet_run(
        &self,
        input: InputId,
        run: RunId,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE handoff_packets SET run_id = ? WHERE input_id = ? AND run_id IS NULL \
             AND state = 'Sent'",
        )
        .bind(to_sql_int(run.0))
        .bind(to_sql_int(input.0))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 크래시로 결과를 받지 못한 시도는 보냈는지 모르는 것으로 확정한다. 다시 보내지 않는다.
    ///
    /// # Errors
    /// 쓰기 실패면 `Database`.
    pub(crate) async fn mark_unsettled_packets_unknown(&self) -> Result<u64, StoreError> {
        let done =
            sqlx::query("UPDATE handoff_packets SET state = 'Unknown' WHERE state = 'Prepared'")
                .execute(&self.pool)
                .await?;
        Ok(done.rows_affected())
    }

    /// 채팅의 패킷 시도를 만든 순서로 읽는다.
    ///
    /// # Errors
    /// 읽기 실패면 `Database`.
    pub(crate) async fn packets_of_chat(
        &self,
        chat: ChatId,
    ) -> Result<Vec<StoredPacket>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, kind, attempt, reduced_from, session_id, input_id, run_id, provider, \
             provider_session, body_hash, body_bytes, state, requested_selector, actual_selector, \
             selection_fallback, candidate_omitted_bytes, state_overflow_bytes FROM handoff_packets \
             WHERE chat_id = ? ORDER BY id",
        )
        .bind(to_sql_int(chat.0))
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(StoredPacket {
                    id: PacketId(from_sql_int(row.try_get("id")?)),
                    kind: parse_enum(&row.try_get::<String, _>("kind")?)?,
                    attempt: u32::try_from(row.try_get::<i64, _>("attempt")?).unwrap_or(u32::MAX),
                    reduced_from: row
                        .try_get::<Option<i64>, _>("reduced_from")?
                        .map(|id| PacketId(from_sql_int(id))),
                    session: SessionId(from_sql_int(row.try_get("session_id")?)),
                    input: row
                        .try_get::<Option<i64>, _>("input_id")?
                        .map(|id| InputId(from_sql_int(id))),
                    run: row
                        .try_get::<Option<i64>, _>("run_id")?
                        .map(|id| RunId(from_sql_int(id))),
                    provider: parse_enum(&row.try_get::<String, _>("provider")?)?,
                    provider_session: row.try_get("provider_session")?,
                    body_hash: row.try_get("body_hash")?,
                    body_bytes: from_sql_int(row.try_get("body_bytes")?),
                    state: parse_enum(&row.try_get::<String, _>("state")?)?,
                    selection: selection_of(row)?,
                })
            })
            .collect()
    }

    /// 시도 하나의 항목을 기록한 순서로 읽는다. 값은 구역, 번호, 들어간 모양, 빠진 이유다.
    ///
    /// # Errors
    /// 읽기 실패면 `Database`.
    pub(crate) async fn packet_items(
        &self,
        id: PacketId,
    ) -> Result<Vec<(String, u64, Option<String>, Option<String>)>, StoreError> {
        let rows = sqlx::query(
            "SELECT zone, ref_id, form, reason FROM handoff_packet_items WHERE packet_id = ? ORDER BY rowid",
        )
        .bind(to_sql_int(id.0))
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok((
                    row.try_get("zone")?,
                    from_sql_int(row.try_get("ref_id")?),
                    row.try_get("form")?,
                    row.try_get("reason")?,
                ))
            })
            .collect()
    }

    /// 시도의 대화 본문 항목마다 `(구역, 번호, 원문 해시)`. 기록한 순서가 실제 순서다.
    ///
    /// # Errors
    /// 읽기 실패면 `Database`.
    #[cfg(test)]
    pub(crate) async fn packet_dialogue(
        &self,
        id: PacketId,
    ) -> Result<Vec<(String, u64, String)>, StoreError> {
        let rows = sqlx::query(
            "SELECT zone, ref_id, body_hash FROM handoff_packet_items \
             WHERE packet_id = ? AND body_hash IS NOT NULL ORDER BY rowid",
        )
        .bind(to_sql_int(id.0))
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok((
                    row.try_get("zone")?,
                    from_sql_int(row.try_get("ref_id")?),
                    row.try_get("body_hash")?,
                ))
            })
            .collect()
    }

    /// 시도의 경쟁 구역 항목마다 `(기록 번호, 고른 방식, 들어간 모양)`. 시험이 선별 방식을 본다.
    #[cfg(test)]
    pub(crate) async fn packet_competing(
        &self,
        id: PacketId,
    ) -> Result<Vec<(u64, String, Option<String>)>, StoreError> {
        let rows = sqlx::query(
            "SELECT ref_id, selector, form FROM handoff_packet_items WHERE packet_id = ? AND zone = 'Competing' ORDER BY rowid",
        )
        .bind(to_sql_int(id.0))
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok((
                    from_sql_int(row.try_get("ref_id")?),
                    row.try_get("selector")?,
                    row.try_get("form")?,
                ))
            })
            .collect()
    }
}
