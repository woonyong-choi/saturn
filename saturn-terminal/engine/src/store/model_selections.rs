//! 모델 정하기 기록과 내보내기에 붙는 적용 결과. 판단 기록 한 건에 하나이고 판단 기록과 함께 지운다.
//! 설계: docs/design/records.md

use std::collections::HashMap;

use saturn_protocol::ids::{ChatId, InputId, JudgmentId};
use serde_json::{Value, json};
use sqlx::Row;

use super::{Store, StoreError, from_sql_int, sha256_hex, to_sql_int};

/// 새 작업의 모델을 어느 규칙이 정했는지.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SelectionSource {
    /// 사용자가 `/model`로 명시 고정했다.
    Pinned,
    /// router의 `target_model` 선택이다. 오토 모드 실험 옵션이다.
    Router,
    /// 사용자 선호에서 품질을 확정한 후보다.
    Preference,
    Default,
    /// 정하지 않아 현재 모델이나 provider 기본값을 쓴다.
    Current,
}

impl SelectionSource {
    fn text(self) -> &'static str {
        match self {
            Self::Pinned => "pinned",
            Self::Router => "router",
            Self::Preference => "preference",
            Self::Default => "default",
            Self::Current => "current",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SelectionIds {
    pub judgment: JudgmentId,
    pub input: InputId,
    pub chat: ChatId,
}

#[derive(Debug, Clone)]
pub(crate) struct NewModelSelection {
    pub ids: SelectionIds,
    pub source: SelectionSource,
    pub model: Option<String>,
    /// router 선택을 쓰지 못한 이유.
    pub reason: Option<&'static str>,
    pub skipped_preferences: Vec<(String, &'static str)>,
    pub candidates_hash: String,
    pub policy: String,
    pub catalog_version: String,
    /// 판단이 어긋나 버려졌으면 거짓.
    pub applied: bool,
}

impl Store {
    /// 한 판단에 한 번만 쓴다.
    pub(crate) async fn record_model_selection(
        &self,
        selection: &NewModelSelection,
    ) -> Result<(), StoreError> {
        let skipped: Vec<Value> = selection
            .skipped_preferences
            .iter()
            .map(|(model, reason)| json!({ "model": model, "reason": reason }))
            .collect();
        sqlx::query(
            "INSERT OR IGNORE INTO model_selections (judgment_id, input_id, chat_id, source, model, reason, \
             skipped_preferences, candidates_hash, policy_digest, catalog_version, applied) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(to_sql_int(selection.ids.judgment.0))
        .bind(to_sql_int(selection.ids.input.0))
        .bind(to_sql_int(selection.ids.chat.0))
        .bind(selection.source.text())
        .bind(&selection.model)
        .bind(selection.reason)
        .bind(serde_json::to_string(&skipped)?)
        .bind(&selection.candidates_hash)
        .bind(&selection.policy)
        .bind(&selection.catalog_version)
        .bind(selection.applied)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 판단 번호별 모델 정하기 기록. 내보내기가 판단 줄에 붙인다.
    pub(super) async fn model_selections_by_judgment(
        &self,
    ) -> Result<HashMap<i64, Value>, StoreError> {
        let rows = sqlx::query("SELECT * FROM model_selections")
            .fetch_all(&self.pool)
            .await?;
        let mut by_judgment = HashMap::new();
        for row in &rows {
            let skipped: Value = serde_json::from_str(row.try_get("skipped_preferences")?)?;
            by_judgment.insert(
                row.try_get::<i64, _>("judgment_id")?,
                json!({
                    "source": row.try_get::<String, _>("source")?,
                    "model": row.try_get::<Option<String>, _>("model")?,
                    "reason": row.try_get::<Option<String>, _>("reason")?,
                    "skipped_preferences": skipped,
                    "candidates_hash": row.try_get::<String, _>("candidates_hash")?,
                    "policy": row.try_get::<String, _>("policy_digest")?,
                    "catalog": row.try_get::<String, _>("catalog_version")?,
                    "applied": row.try_get::<bool, _>("applied")?,
                }),
            );
        }
        Ok(by_judgment)
    }

    /// 입력이 실제로 낳은 것: 입력에 남은 모델, 원문 해시, 연 실행과 session의 모델과 캐시 토큰, 받은 인계 패킷.
    /// 판단 기록과 같은 입력 번호로 이어 붙이는 읽기 전용 조회라 따로 저장하지 않는다.
    pub(super) async fn applied_of_input(&self, input: i64) -> Result<Value, StoreError> {
        let Some(row) = sqlx::query("SELECT text, pinned_model FROM inputs WHERE id = ?")
            .bind(input)
            .fetch_optional(&self.pool)
            .await?
        else {
            return Ok(Value::Null);
        };
        let text: String = row.try_get("text")?;
        let runs = sqlx::query(
            "SELECT r.id, r.provider, r.session_id, s.model AS session_model, \
             SUM(u.input_tokens) AS input_tokens, SUM(u.cache_read_tokens) AS cache_read, \
             SUM(u.cache_write_tokens) AS cache_write \
             FROM runs r LEFT JOIN sessions s ON s.id = r.session_id LEFT JOIN usage u ON u.run_id = r.id \
             WHERE r.input_id = ? GROUP BY r.id ORDER BY r.id",
        )
        .bind(input)
        .fetch_all(&self.pool)
        .await?;
        let mut run_lines = Vec::new();
        for run in &runs {
            run_lines.push(json!({
                "run": run.try_get::<i64, _>("id")?,
                "provider": run.try_get::<String, _>("provider")?,
                "session": run.try_get::<i64, _>("session_id")?,
                "session_model": run.try_get::<Option<String>, _>("session_model")?,
                "input_tokens": run.try_get::<Option<i64>, _>("input_tokens")?,
                "cache_read_tokens": run.try_get::<Option<i64>, _>("cache_read")?,
                "cache_write_tokens": run.try_get::<Option<i64>, _>("cache_write")?,
            }));
        }
        let packets = sqlx::query(
            "SELECT id, kind, state, session_id, chat_revision, settings_revision, policy, body_hash \
             FROM handoff_packets WHERE input_id = ? ORDER BY id",
        )
        .bind(input)
        .fetch_all(&self.pool)
        .await?;
        let mut packet_lines = Vec::new();
        for packet in &packets {
            packet_lines.push(json!({
                "packet": packet.try_get::<i64, _>("id")?,
                "kind": packet.try_get::<String, _>("kind")?,
                "state": packet.try_get::<String, _>("state")?,
                "session": packet.try_get::<i64, _>("session_id")?,
                "chat_revision": from_sql_int(packet.try_get::<i64, _>("chat_revision")?),
                "settings_revision": from_sql_int(packet.try_get::<i64, _>("settings_revision")?),
                "policy": packet.try_get::<String, _>("policy")?,
                "body_hash": packet.try_get::<String, _>("body_hash")?,
            }));
        }
        Ok(json!({
            "input_model": row.try_get::<Option<String>, _>("pinned_model")?,
            "input_sha256": sha256_hex(text.as_bytes()),
            "runs": run_lines,
            "packets": packet_lines,
        }))
    }
}
