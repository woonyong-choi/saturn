//! 모델 판단 그림자 기록. 판단 기록 한 건에 하나이고 판단 기록과 함께 지우고 내보낸다.
//! 설계: docs/design/records.md

use std::collections::HashMap;

use saturn_core::routers::QuestionSetId;
use saturn_protocol::ids::{ChatId, ChatRevision, InputId, JudgmentId, SettingsRevision};
use serde_json::{Value, json};
use sqlx::Row;

use super::{Store, StoreError, from_sql_int, to_sql_int};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShadowStatus {
    Answered,
    /// 답이 형식에 맞지 않았다.
    Invalid,
    /// 호출이 실패했거나 답이 없다.
    NoAnswer,
    /// 판단이 어긋나 적용하지 않았다.
    Superseded,
}

impl ShadowStatus {
    fn text(self) -> &'static str {
        match self {
            Self::Answered => "answered",
            Self::Invalid => "invalid",
            Self::NoAnswer => "no-answer",
            Self::Superseded => "superseded",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ShadowCandidate {
    /// `<provider>/<model>` 글.
    pub model: String,
    /// 모델 목록 기준 품질 확정 여부. `verified` 또는 `unverified:<이유>`.
    pub quality: String,
    /// 충분할 확률. 답이 없으면 `None`.
    pub probability: Option<f64>,
}

#[derive(Debug, Clone)]
pub(crate) struct NewModelShadow {
    pub judgment: JudgmentId,
    pub input: InputId,
    pub chat: ChatId,
    pub settings: SettingsRevision,
    /// 판단을 시작한 채팅 revision.
    pub chat_revision: ChatRevision,
    /// 정책 지문.
    pub policy: String,
    pub catalog_version: String,
    pub question_set: QuestionSetId,
    pub candidates_hash: String,
    pub candidates: Vec<ShadowCandidate>,
    pub status: ShadowStatus,
    /// 실제로 적용한 모델. 판단이 어긋나 적용하지 않았거나 정하지 않았으면 `None`.
    pub applied_model: Option<String>,
    /// 그림자 질문이 요청에 더한 바이트.
    pub request_bytes: usize,
}

impl Store {
    /// 판단 기록이 없으면(`/record off`) 쓰지 않는다. 한 판단에 한 번만 쓴다.
    pub(crate) async fn record_model_shadow(
        &self,
        shadow: &NewModelShadow,
    ) -> Result<(), StoreError> {
        let candidates: Vec<Value> = shadow
            .candidates
            .iter()
            .map(|candidate| {
                json!({
                    "model": candidate.model,
                    "quality": candidate.quality,
                    "probability": candidate.probability,
                })
            })
            .collect();
        sqlx::query(
            "INSERT OR IGNORE INTO model_shadows (judgment_id, input_id, chat_id, settings_revision, chat_revision, \
             policy_digest, catalog_version, question_set, candidates_hash, candidates, status, applied_model, \
             request_bytes) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(to_sql_int(shadow.judgment.0))
        .bind(to_sql_int(shadow.input.0))
        .bind(to_sql_int(shadow.chat.0))
        .bind(to_sql_int(shadow.settings.0))
        .bind(to_sql_int(shadow.chat_revision.0))
        .bind(&shadow.policy)
        .bind(&shadow.catalog_version)
        .bind(format!(
            "{}@{}.{}",
            shadow.question_set.name, shadow.question_set.major, shadow.question_set.minor
        ))
        .bind(&shadow.candidates_hash)
        .bind(serde_json::to_string(&candidates)?)
        .bind(shadow.status.text())
        .bind(&shadow.applied_model)
        .bind(to_sql_int(shadow.request_bytes as u64))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 판단 번호별 그림자 기록. 내보내기가 판단 줄에 붙인다.
    pub(super) async fn model_shadows_by_judgment(
        &self,
    ) -> Result<HashMap<i64, Value>, StoreError> {
        let rows = sqlx::query("SELECT * FROM model_shadows")
            .fetch_all(&self.pool)
            .await?;
        let mut by_judgment = HashMap::new();
        for row in &rows {
            let candidates: Value = serde_json::from_str(row.try_get("candidates")?)?;
            by_judgment.insert(
                row.try_get::<i64, _>("judgment_id")?,
                json!({
                    "question_set": row.try_get::<String, _>("question_set")?,
                    "status": row.try_get::<String, _>("status")?,
                    "policy": row.try_get::<String, _>("policy_digest")?,
                    "catalog": row.try_get::<String, _>("catalog_version")?,
                    "settings": row.try_get::<i64, _>("settings_revision")?,
                    "chat_revision": row.try_get::<i64, _>("chat_revision")?,
                    "candidates_hash": row.try_get::<String, _>("candidates_hash")?,
                    "candidates": candidates,
                    "applied_model": row.try_get::<Option<String>, _>("applied_model")?,
                    "request_bytes": from_sql_int(row.try_get::<i64, _>("request_bytes")?),
                }),
            );
        }
        Ok(by_judgment)
    }
}
