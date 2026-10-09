//! 채팅, 입력, 실행, session, provider 이벤트, 사용량 원값 기록.
//! 설계: docs/design/records.md

use std::path::PathBuf;
use std::time::SystemTime;

use rows::{parse_permission, permission_text};
use saturn_core::queue::Permission;
use saturn_protocol::event::UsageReport;
use saturn_protocol::ids::{
    AgentId, ChatId, InputId, Provider, RunId, SessionId, SettingsRevision, TaskId,
};
use saturn_protocol::state::{EffectScope, InputState, QueueReason};
use sqlx::Row;

use super::{Store, StoreError, enum_text, from_sql_int, parse_enum, to_millis, to_sql_int};

mod rows;
mod runs;

#[cfg(test)]
pub(in crate::store) use rows::unknown_value;
pub(crate) use rows::{ensure_found, not_found};
pub(in crate::store) use rows::{parse_run_end, session_from_row};

/// 이 밖의 입력은 열린 입력이다.
pub(crate) const FINAL_INPUT_STATES: &str = "('Applied', 'Rejected', 'Cancelled')";

/// 접수 때 설정 번호와 권한을 고정하고 끝까지 바꾸지 않는다.
#[derive(Debug, Clone)]
pub(crate) struct NewInput {
    pub chat: ChatId,
    /// 사용자 입력이고 router 키가 아니라 마스킹하지 않는다.
    pub text: String,
    pub settings: SettingsRevision,
    pub permission: Permission,
    pub workdir: PathBuf,
    pub pinned_model: Option<String>,
    /// `Tab`으로 관계 판단 없이 대기.
    pub skip_relation: bool,
}

/// provider 턴 하나.
#[derive(Debug, Clone)]
pub(crate) struct NewRun {
    /// `provider-wake` 턴이면 `None`.
    pub input: Option<InputId>,
    pub task: TaskId,
    pub agent: AgentId,
    pub session: SessionId,
    pub provider: Provider,
    /// 적용된 provider 설정으로 증명되면 `ProvenByConfig`, 아니면 `NetworkPossible`에서 시작한다.
    pub effect_scope: EffectScope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunEnd {
    Completed,
    Failed,
    Stopped,
}

#[derive(Debug, Clone)]
pub(crate) struct RunRecord {
    pub id: RunId,
    pub chat: ChatId,
    pub agent: AgentId,
    pub input: Option<InputId>,
    pub task: TaskId,
    pub session: SessionId,
    /// `allows_auto_resume()`이 참일 때만 자동 재개한다.
    pub effect_scope: EffectScope,
    /// 끝나지 않았으면 `None`(열린 실행).
    #[cfg(test)]
    pub end: Option<RunEnd>,
}

/// 보고하지 않은 칸은 `None`이고 합계에서 0으로 세지 않는다.
#[derive(Debug, Clone)]
pub(crate) struct UsageRow {
    /// 기록 순서.
    pub id: u64,
    pub run: RunId,
    /// `ThreadCumulative`는 같은 session의 직전 누적을 빼서 턴 값을 구한다.
    pub session: SessionId,
    /// 그 실행의 provider.
    pub provider: Provider,
    pub report: UsageReport,
    /// 중간 보고가 빠져 이 값의 차이가 여러 턴에 걸친다. `/usage` 응답은 protocol `UsageRow::turns`로 턴 수를 싣는다.
    #[cfg(test)]
    pub spans_turns: bool,
}

impl Store {
    /// 판단 기록 저장은 켜진 상태로 시작한다.
    pub(crate) async fn create_chat(&self, workdir: PathBuf) -> Result<ChatId, StoreError> {
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO chats (workdir, recording, created_at) VALUES (?, 1, ?) RETURNING id",
        )
        .bind(workdir.to_string_lossy().into_owned())
        .bind(to_millis(SystemTime::now()))
        .fetch_one(&self.pool)
        .await?;
        Ok(ChatId(from_sql_int(id)))
    }

    /// 채팅을 만들 때 고정한 작업 폴더.
    ///
    /// # Errors
    /// 없는 채팅이면 `NotFound`.
    pub(crate) async fn chat_workdir(&self, chat: ChatId) -> Result<PathBuf, StoreError> {
        let row: Option<String> = sqlx::query_scalar("SELECT workdir FROM chats WHERE id = ?")
            .bind(to_sql_int(chat.0))
            .fetch_optional(&self.pool)
            .await?;
        row.map(PathBuf::from)
            .ok_or_else(|| not_found(format!("chat {}", chat.0)))
    }

    /// 끄면 새 판단 기록만 저장하지 않고 이미 저장한 것은 둔다.
    ///
    /// # Errors
    /// 없는 채팅이면 `NotFound`.
    pub(crate) async fn set_recording(&self, chat: ChatId, on: bool) -> Result<(), StoreError> {
        let done = sqlx::query("UPDATE chats SET recording = ? WHERE id = ?")
            .bind(on)
            .bind(to_sql_int(chat.0))
            .execute(&self.pool)
            .await?;
        ensure_found(done.rows_affected(), || format!("chat {}", chat.0))
    }

    /// 채팅의 고정 모델 글. 다시 바꿀 때까지 이어지고 다음 입력부터 쓴다.
    ///
    /// # Errors
    /// 없는 채팅이면 `NotFound`.
    pub(crate) async fn set_chat_model(&self, chat: ChatId, model: &str) -> Result<(), StoreError> {
        let done = sqlx::query("UPDATE chats SET pinned_model = ? WHERE id = ?")
            .bind(model)
            .bind(to_sql_int(chat.0))
            .execute(&self.pool)
            .await?;
        ensure_found(done.rows_affected(), || format!("chat {}", chat.0))
    }

    /// 고정하지 않았으면 `None`.
    ///
    /// # Errors
    /// 없는 채팅이면 `NotFound`.
    pub(crate) async fn chat_model(&self, chat: ChatId) -> Result<Option<String>, StoreError> {
        let row: Option<Option<String>> =
            sqlx::query_scalar("SELECT pinned_model FROM chats WHERE id = ?")
                .bind(to_sql_int(chat.0))
                .fetch_optional(&self.pool)
                .await?;
        row.ok_or_else(|| not_found(format!("chat {}", chat.0)))
    }

    /// 보조 에이전트도 같은 채팅이라 부모 채팅의 값을 그대로 읽는다.
    ///
    /// # Errors
    /// 없는 채팅이면 `NotFound`.
    pub(crate) async fn chat_layer(&self, chat: ChatId) -> Result<Option<String>, StoreError> {
        let row: Option<Option<String>> =
            sqlx::query_scalar("SELECT chat_layer FROM chats WHERE id = ?")
                .bind(to_sql_int(chat.0))
                .fetch_optional(&self.pool)
                .await?;
        row.ok_or_else(|| not_found(format!("chat {}", chat.0)))
    }

    /// 원본 파일이 없는 층이라 여기가 정본이다.
    ///
    /// # Errors
    /// 없는 채팅이면 `NotFound`.
    pub(crate) async fn set_chat_layer(&self, chat: ChatId, toml: &str) -> Result<(), StoreError> {
        let done = sqlx::query("UPDATE chats SET chat_layer = ? WHERE id = ?")
            .bind(toml)
            .bind(to_sql_int(chat.0))
            .execute(&self.pool)
            .await?;
        ensure_found(done.rows_affected(), || format!("chat {}", chat.0))
    }

    /// 이 함수가 성공한 뒤에만 `Queue::accept`를 부르고 TUI에 에코를 보낸다.
    ///
    /// # Errors
    /// 쓰기 실패면 `Database`(없는 채팅 포함)이고 입력은 접수되지 않았다.
    pub(crate) async fn accept_input(&self, input: &NewInput) -> Result<InputId, StoreError> {
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO inputs (chat_id, text, settings_revision, permission, workdir, pinned_model, \
             skip_relation, state, reason, accepted_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL, ?) RETURNING id",
        )
        .bind(to_sql_int(input.chat.0))
        .bind(&input.text)
        .bind(to_sql_int(input.settings.0))
        .bind(permission_text(input.permission))
        .bind(input.workdir.to_string_lossy().into_owned())
        .bind(&input.pinned_model)
        .bind(input.skip_relation)
        .bind(enum_text(&InputState::Judging)?)
        .bind(to_millis(SystemTime::now()))
        .fetch_one(&self.pool)
        .await?;
        Ok(InputId(from_sql_int(id)))
    }

    /// 전이 규칙 검사는 `core::queue`가 먼저 한다.
    ///
    /// # Errors
    /// 없는 입력이면 `NotFound`.
    pub(crate) async fn set_input_state(
        &self,
        input: InputId,
        state: InputState,
        reason: Option<QueueReason>,
    ) -> Result<(), StoreError> {
        let reason = reason
            .map(|reason| serde_json::to_string(&reason))
            .transpose()?;
        let done = sqlx::query("UPDATE inputs SET state = ?, reason = ? WHERE id = ?")
            .bind(enum_text(&state)?)
            .bind(reason)
            .bind(to_sql_int(input.0))
            .execute(&self.pool)
            .await?;
        ensure_found(done.rows_affected(), || format!("input {}", input.0))
    }

    /// router가 고른 모델을 입력에 남긴다. 다시 시작해도 같은 모델로 보내기 위해서다.
    ///
    /// # Errors
    /// 없는 입력이면 `NotFound`.
    pub(crate) async fn set_input_model(
        &self,
        input: InputId,
        model: Option<&str>,
    ) -> Result<(), StoreError> {
        let done = sqlx::query("UPDATE inputs SET pinned_model = ? WHERE id = ?")
            .bind(model)
            .bind(to_sql_int(input.0))
            .execute(&self.pool)
            .await?;
        ensure_found(done.rows_affected(), || format!("input {}", input.0))
    }

    /// 끝 상태인 입력도 읽는다. 크래시로 끊긴 실행의 입력을 되살리는 데 쓴다.
    ///
    /// # Errors
    /// 없는 입력이면 `NotFound`.
    pub(crate) async fn stored_input(
        &self,
        input: InputId,
    ) -> Result<(NewInput, InputState), StoreError> {
        let row = sqlx::query(
            "SELECT chat_id, text, settings_revision, permission, workdir, pinned_model, skip_relation, state \
             FROM inputs WHERE id = ?",
        )
        .bind(to_sql_int(input.0))
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| not_found(format!("input {}", input.0)))?;
        let input = NewInput {
            chat: ChatId(from_sql_int(row.try_get("chat_id")?)),
            text: row.try_get("text")?,
            settings: SettingsRevision(from_sql_int(row.try_get("settings_revision")?)),
            permission: parse_permission(row.try_get("permission")?)?,
            workdir: PathBuf::from(row.try_get::<String, _>("workdir")?),
            pinned_model: row.try_get("pinned_model")?,
            skip_relation: row.try_get("skip_relation")?,
        };
        Ok((input, parse_enum(row.try_get("state")?)?))
    }

    /// 시작 때 대기열을 되살리는 데 쓴다. 접수 순서.
    pub(crate) async fn open_inputs(
        &self,
    ) -> Result<Vec<(InputId, NewInput, InputState)>, StoreError> {
        let rows = sqlx::query(&format!(
            "SELECT id, chat_id, text, settings_revision, permission, workdir, pinned_model, \
             skip_relation, state FROM inputs WHERE state NOT IN {FINAL_INPUT_STATES} ORDER BY id"
        ))
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                let input = NewInput {
                    chat: ChatId(from_sql_int(row.try_get("chat_id")?)),
                    text: row.try_get("text")?,
                    settings: SettingsRevision(from_sql_int(row.try_get("settings_revision")?)),
                    permission: parse_permission(row.try_get("permission")?)?,
                    workdir: PathBuf::from(row.try_get::<String, _>("workdir")?),
                    pinned_model: row.try_get("pinned_model")?,
                    skip_relation: row.try_get("skip_relation")?,
                };
                let state = parse_enum(row.try_get("state")?)?;
                Ok((InputId(from_sql_int(row.try_get("id")?)), input, state))
            })
            .collect()
    }

    /// 처리 중인 동안 그 채팅은 정리 대상에서 빠지고, 이미 처리 중이면 그대로 둔다.
    pub(crate) async fn begin_stop(&self, chat: ChatId) -> Result<(), StoreError> {
        sqlx::query("INSERT OR IGNORE INTO stops (chat_id, started_at) VALUES (?, ?)")
            .bind(to_sql_int(chat.0))
            .bind(to_millis(SystemTime::now()))
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// # Errors
    /// 처리 중인 요청이 없으면 `NotFound`.
    pub(crate) async fn end_stop(&self, chat: ChatId) -> Result<(), StoreError> {
        let done = sqlx::query("DELETE FROM stops WHERE chat_id = ?")
            .bind(to_sql_int(chat.0))
            .execute(&self.pool)
            .await?;
        ensure_found(done.rows_affected(), || format!("stop of chat {}", chat.0))
    }
}

#[cfg(test)]
pub(crate) mod tests;
