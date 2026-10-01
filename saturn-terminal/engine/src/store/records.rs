//! 채팅, 입력 접수와 상태, 중지 요청, 실행과 `effect_scope`, session, provider 이벤트, 사용량 원값.
//!
//! 설계: docs/design/records.md(기록 저장소), docs/design/input-handling.md(입력 접수), docs/design/engine-lifecycle.md(효과 범위, 크래시 뒤 복구).
//! 메서드마다 거래 하나. 입력은 여기서 접수(ACK)된 뒤에만 `core::queue`로 넘기고 provider로 보낸다.
//! TODO(#31): 표 이름(chats/inputs/runs/sessions/events/usage 초안)을 용어 표에 맞출지

use std::path::PathBuf;
use std::time::SystemTime;

use saturn_core::queue::Permission;
use saturn_core::sessions::{AgentRole, SessionRecord};
use saturn_protocol::event::{ProviderEvent, UsageReport, UsageScope};
use saturn_protocol::ids::{
    AgentId, ChatId, InputId, LedgerSeq, Provider, ProviderSessionId, RunId, SessionId,
    SettingsRevision, TaskId,
};
use saturn_protocol::rpc::UsageRange;
use saturn_protocol::state::{EffectScope, InputState, QueueReason};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use super::{
    Store, StoreError, enum_text, from_millis, from_sql_int, parse_enum, to_millis, to_sql_int,
};

/// 끝 상태 입력의 `state` 칸 값. 이 밖의 입력은 열린 입력이다.
pub(crate) const FINAL_INPUT_STATES: &str = "('Applied', 'Rejected', 'Cancelled')";

/// 접수할 입력. 접수 때 설정 번호와 권한을 고정하고 끝까지 바꾸지 않는다.
#[derive(Debug, Clone)]
pub struct NewInput {
    /// 채팅.
    pub chat: ChatId,
    /// 원문. 마스킹하지 않고 그대로 둔다(사용자 입력이고 judge 키가 아니다).
    pub text: String,
    /// 접수 때 고정한 설정 번호.
    pub settings: SettingsRevision,
    /// 접수 때 고정한 권한.
    pub permission: Permission,
    /// 작업 폴더.
    pub workdir: PathBuf,
    /// 사용자가 고정한 모델.
    pub pinned_model: Option<String>,
    /// `Tab`으로 관계 판단 없이 대기.
    pub skip_relation: bool,
}

/// 시작할 실행 하나(provider 턴 하나).
#[derive(Debug, Clone)]
pub struct NewRun {
    /// 실행을 만든 입력. `provider-wake` 턴이면 `None`.
    pub input: Option<InputId>,
    /// 작업.
    pub task: TaskId,
    /// 에이전트.
    pub agent: AgentId,
    /// session.
    pub session: SessionId,
    /// provider.
    pub provider: Provider,
    /// 시작 때 효과 범위. 적용된 provider 설정으로 증명되면 `ProvenByConfig`, 아니면 `NetworkPossible`에서 시작한다.
    pub effect_scope: EffectScope,
}

/// 실행 끝 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunEnd {
    /// 완료 신호를 받았다.
    Completed,
    /// provider가 실패를 알렸다.
    Failed,
    /// 사용자 멈춤으로 중단했다.
    Stopped,
}

/// 저장된 실행 한 건. 크래시 뒤 복구에서 읽는다.
#[derive(Debug, Clone)]
pub struct RunRecord {
    /// 실행 id.
    pub id: RunId,
    /// 만든 입력.
    pub input: Option<InputId>,
    /// 작업.
    pub task: TaskId,
    /// session.
    pub session: SessionId,
    /// 마지막으로 기록한 효과 범위. `allows_auto_resume()`이 참일 때만 자동 재개한다.
    pub effect_scope: EffectScope,
    /// 시작 시각.
    pub started_at: SystemTime,
    /// 끝 결과. 끝나지 않았으면 `None`(열린 실행).
    pub end: Option<RunEnd>,
}

/// 사용량 원값 한 행. 보고하지 않은 칸은 `None`이고 합계에서 0으로 세지 않는다.
#[derive(Debug, Clone)]
pub struct UsageRow {
    /// 실행.
    pub run: RunId,
    /// session. `ThreadCumulative`는 같은 session의 직전 누적을 빼서 턴 값을 구한다.
    pub session: SessionId,
    /// 원값 그대로.
    pub report: UsageReport,
    /// 받은 시각.
    pub at: SystemTime,
    /// 중간 보고가 빠져 이 값의 차이가 여러 턴에 걸친다. 기록 저장소는 참·거짓만 두고 화면 문구는 정하지 않는다.
    /// TODO(#90): `/usage` 응답에 이 값을 TUI로 넘기는 protocol 필드를 더하고 화면 문구(초안 `여러 턴 합계`)를 정한다
    pub spans_turns: bool,
}

impl Store {
    /// 새 채팅을 만든다. 판단 기록 저장은 켜진 상태로 시작한다.
    ///
    /// # Errors
    /// 쓰기 실패면 `Database`.
    pub async fn create_chat(&self, workdir: PathBuf) -> Result<ChatId, StoreError> {
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO chats (workdir, recording, created_at) VALUES (?, 1, ?) RETURNING id",
        )
        .bind(workdir.to_string_lossy().into_owned())
        .bind(to_millis(SystemTime::now()))
        .fetch_one(&self.pool)
        .await?;
        Ok(ChatId(from_sql_int(id)))
    }

    /// `/record off`·`/record on`. 끄면 그 채팅의 판단 기록을 새로 저장하지 않는다(이미 저장한 것은 두고).
    ///
    /// # Errors
    /// 없는 채팅이면 `NotFound`.
    pub async fn set_recording(&self, chat: ChatId, on: bool) -> Result<(), StoreError> {
        let done = sqlx::query("UPDATE chats SET recording = ? WHERE id = ?")
            .bind(on)
            .bind(to_sql_int(chat.0))
            .execute(&self.pool)
            .await?;
        ensure_found(done.rows_affected(), || format!("chat {}", chat.0))
    }

    /// 채팅 층 설정(TOML 원문). 없으면 `None`. 보조 에이전트도 같은 채팅이므로 부모 채팅의 값을 그대로 읽는다.
    ///
    /// # Errors
    /// 없는 채팅이면 `NotFound`.
    pub async fn chat_layer(&self, chat: ChatId) -> Result<Option<String>, StoreError> {
        let row: Option<Option<String>> =
            sqlx::query_scalar("SELECT chat_layer FROM chats WHERE id = ?")
                .bind(to_sql_int(chat.0))
                .fetch_optional(&self.pool)
                .await?;
        row.ok_or_else(|| not_found(format!("chat {}", chat.0)))
    }

    /// 채팅 층 설정을 바꾼다. 원본 파일이 없는 층이라 여기가 정본이다.
    ///
    /// # Errors
    /// 없는 채팅이면 `NotFound`.
    pub async fn set_chat_layer(&self, chat: ChatId, toml: &str) -> Result<(), StoreError> {
        let done = sqlx::query("UPDATE chats SET chat_layer = ? WHERE id = ?")
            .bind(toml)
            .bind(to_sql_int(chat.0))
            .execute(&self.pool)
            .await?;
        ensure_found(done.rows_affected(), || format!("chat {}", chat.0))
    }

    /// 입력 접수(ACK). 한 거래로 입력 행을 쓰고 상태 `Judging`으로 둔 뒤 새 `InputId`를 돌려준다.
    /// 이 함수가 성공한 뒤에만 `Queue::accept`를 부르고 TUI에 에코를 보낸다.
    ///
    /// # Errors
    /// 쓰기 실패면 `Database`(없는 채팅 포함). 이때 입력은 접수되지 않았고 TUI에 실패를 알린다.
    pub async fn accept_input(&self, input: &NewInput) -> Result<InputId, StoreError> {
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

    /// 입력 상태와 대기 이유를 바꾼다. 전이 규칙 검사는 `core::queue`가 먼저 한다.
    ///
    /// # Errors
    /// 없는 입력이면 `NotFound`.
    pub async fn set_input_state(
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

    /// 끝 상태(`Applied`, `Rejected`, `Cancelled`)가 아닌 입력. 시작 때 대기열을 되살리는 데 쓴다. 접수 순서.
    ///
    /// # Errors
    /// 조회 실패면 `Database`.
    pub async fn open_inputs(&self) -> Result<Vec<(InputId, NewInput, InputState)>, StoreError> {
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

    /// 멈춤(`Ctrl+C`) 요청을 처리 중으로 기록한다. 처리 중인 동안 그 채팅은 정리 대상에서 빠진다. 채팅마다 동시에 하나.
    /// 이미 처리 중이면 그대로 둔다.
    ///
    /// # Errors
    /// 쓰기 실패면 `Database`.
    pub async fn begin_stop(&self, chat: ChatId) -> Result<(), StoreError> {
        sqlx::query("INSERT OR IGNORE INTO stops (chat_id, started_at) VALUES (?, ?)")
            .bind(to_sql_int(chat.0))
            .bind(to_millis(SystemTime::now()))
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// 멈춤 처리 끝. 보류 결과는 입력과 session 상태에 이미 반영되어 있다.
    ///
    /// # Errors
    /// 처리 중인 요청이 없으면 `NotFound`.
    pub async fn end_stop(&self, chat: ChatId) -> Result<(), StoreError> {
        let done = sqlx::query("DELETE FROM stops WHERE chat_id = ?")
            .bind(to_sql_int(chat.0))
            .execute(&self.pool)
            .await?;
        ensure_found(done.rows_affected(), || format!("stop of chat {}", chat.0))
    }

    /// 실행 시작을 기록하고 `RunId`를 돌려준다. provider에 보내기 전에 부른다.
    /// 채팅은 입력이 있으면 입력의 채팅, 없으면(`provider-wake`) session 행의 채팅이다.
    ///
    /// # Errors
    /// 쓰기 실패면 `Database`, 입력도 session 행도 없어 채팅을 정하지 못하면 `NotFound`. 이때 provider로 보내지 않는다.
    pub async fn start_run(&self, run: &NewRun) -> Result<RunId, StoreError> {
        let mut tx = self.pool.begin().await?;
        let chat: Option<i64> = match run.input {
            Some(input) => {
                sqlx::query_scalar("SELECT chat_id FROM inputs WHERE id = ?")
                    .bind(to_sql_int(input.0))
                    .fetch_optional(&mut *tx)
                    .await?
            }
            None => {
                sqlx::query_scalar("SELECT chat_id FROM sessions WHERE id = ?")
                    .bind(to_sql_int(run.session.0))
                    .fetch_optional(&mut *tx)
                    .await?
            }
        };
        let chat = chat.ok_or_else(|| match run.input {
            Some(input) => not_found(format!("input {}", input.0)),
            None => not_found(format!("session {}", run.session.0)),
        })?;
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO runs (chat_id, input_id, task_id, agent_id, session_id, provider, effect_scope, \
             started_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(chat)
        .bind(run.input.map(|input| to_sql_int(input.0)))
        .bind(to_sql_int(run.task.0))
        .bind(to_sql_int(run.agent.0))
        .bind(to_sql_int(run.session.0))
        .bind(enum_text(&run.provider)?)
        .bind(enum_text(&run.effect_scope)?)
        .bind(to_millis(SystemTime::now()))
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(RunId(from_sql_int(id)))
    }

    /// 효과 범위를 바꾼다. 관찰이 끊기면(`StreamLost`, 읽는 중 종료, 끝이 없는 subagent) `Unobserved`로 바꾸고 되돌리지 않는다.
    /// 이미 `Unobserved`인 실행은 다른 값으로 바꾸지 않는다.
    ///
    /// # Errors
    /// 없는 실행이면 `NotFound`.
    pub async fn set_effect_scope(&self, run: RunId, scope: EffectScope) -> Result<(), StoreError> {
        let unobserved = enum_text(&EffectScope::Unobserved)?;
        let done = sqlx::query(
            "UPDATE runs SET effect_scope = CASE WHEN effect_scope = ? THEN effect_scope ELSE ? END \
             WHERE id = ?",
        )
        .bind(unobserved)
        .bind(enum_text(&scope)?)
        .bind(to_sql_int(run.0))
        .execute(&self.pool)
        .await?;
        ensure_found(done.rows_affected(), || format!("run {}", run.0))
    }

    /// 실행 끝을 기록하고 같은 거래 뒤에 원시 기록을 gzip으로 압축한다(`raw::compress_run`).
    /// 이미 끝난 실행이면 처음 기록한 끝을 그대로 둔다.
    ///
    /// # Errors
    /// 없는 실행이면 `NotFound`, 압축 실패면 `Compression`(끝 기록은 남고 원시 기록은 압축 전 그대로).
    pub async fn finish_run(&self, run: RunId, end: RunEnd) -> Result<(), StoreError> {
        let done = sqlx::query(
            "UPDATE runs SET end_kind = COALESCE(end_kind, ?), ended_at = COALESCE(ended_at, ?) \
             WHERE id = ?",
        )
        .bind(run_end_text(end))
        .bind(to_millis(SystemTime::now()))
        .bind(to_sql_int(run.0))
        .execute(&self.pool)
        .await?;
        ensure_found(done.rows_affected(), || format!("run {}", run.0))?;
        self.compress_run(run).await?;
        Ok(())
    }

    /// 끝나지 않은 실행. 크래시 뒤 복구에서 `effect_scope`로 자동 재개와 보류를 나눈다.
    ///
    /// # Errors
    /// 조회 실패면 `Database`.
    pub async fn unfinished_runs(&self) -> Result<Vec<RunRecord>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, input_id, task_id, session_id, effect_scope, started_at, end_kind FROM runs \
             WHERE end_kind IS NULL ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(run_record).collect()
    }

    /// session 행을 넣거나 고친다. `provider_session`, 상태, `delivered`를 쓴다. `idle_since`(`Instant`)는 저장하지 않는다.
    ///
    /// # Errors
    /// 쓰기 실패면 `Database`.
    pub async fn upsert_session(&self, session: &SessionRecord) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO sessions (id, chat_id, agent_id, role, provider, provider_session, state, delivered) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET \
             provider_session = excluded.provider_session, state = excluded.state, \
             delivered = excluded.delivered",
        )
        .bind(to_sql_int(session.id.0))
        .bind(to_sql_int(session.chat.0))
        .bind(to_sql_int(session.agent.0))
        .bind(role_text(session.role))
        .bind(enum_text(&session.provider)?)
        .bind(session.provider_session.as_ref().map(|id| id.0.clone()))
        .bind(enum_text(&session.state)?)
        .bind(to_sql_int(session.delivered.0))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 채팅의 session 목록. 닫은 session의 provider session id로 재개할 때 쓴다. id 순서.
    ///
    /// # Errors
    /// 조회 실패면 `Database`.
    pub async fn sessions(&self, chat: ChatId) -> Result<Vec<SessionRecord>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, chat_id, agent_id, role, provider, provider_session, state, delivered \
             FROM sessions WHERE chat_id = ? ORDER BY id",
        )
        .bind(to_sql_int(chat.0))
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(SessionRecord {
                    id: SessionId(from_sql_int(row.try_get("id")?)),
                    chat: ChatId(from_sql_int(row.try_get("chat_id")?)),
                    agent: AgentId(from_sql_int(row.try_get("agent_id")?)),
                    role: parse_role(row.try_get("role")?)?,
                    provider: parse_enum(row.try_get("provider")?)?,
                    provider_session: row
                        .try_get::<Option<String>, _>("provider_session")?
                        .map(ProviderSessionId),
                    state: parse_enum(row.try_get("state")?)?,
                    delivered: LedgerSeq(from_sql_int(row.try_get("delivered")?)),
                    idle_since: None,
                })
            })
            .collect()
    }

    /// provider 이벤트를 채팅 기록에 붙이고 새 `LedgerSeq`를 돌려준다. 번호는 채팅 트리 전체에서 1부터 1씩 늘어난다.
    /// 이벤트는 JSON으로 저장한다. `Usage` 이벤트는 여기 말고 `record_usage`로 쓴다.
    ///
    /// # Errors
    /// 없는 실행이면 `NotFound`, 직렬화 실패면 `Json`.
    pub async fn append_event(
        &self,
        run: RunId,
        chat: ChatId,
        event: &ProviderEvent,
    ) -> Result<LedgerSeq, StoreError> {
        let body = serde_json::to_string(event)?;
        let mut tx = self.pool.begin().await?;
        let exists: Option<i64> = sqlx::query_scalar("SELECT id FROM runs WHERE id = ?")
            .bind(to_sql_int(run.0))
            .fetch_optional(&mut *tx)
            .await?;
        if exists.is_none() {
            return Err(not_found(format!("run {}", run.0)));
        }
        let seq: i64 = sqlx::query_scalar(
            "INSERT INTO events (chat_id, seq, run_id, body, at) \
             VALUES (?1, (SELECT COALESCE(MAX(seq), 0) + 1 FROM events WHERE chat_id = ?1), ?2, ?3, ?4) \
             RETURNING seq",
        )
        .bind(to_sql_int(chat.0))
        .bind(to_sql_int(run.0))
        .bind(body)
        .bind(to_millis(SystemTime::now()))
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(LedgerSeq(from_sql_int(seq)))
    }

    /// `after` 뒤의 이벤트를 번호 순서로. session에 변경분만 첨부하거나 TUI가 다시 붙을 때 쓴다.
    ///
    /// # Errors
    /// 조회 실패면 `Database`, 저장된 JSON이 깨졌으면 `Json`.
    pub async fn events_since(
        &self,
        chat: ChatId,
        after: LedgerSeq,
    ) -> Result<Vec<(LedgerSeq, ProviderEvent)>, StoreError> {
        let rows =
            sqlx::query("SELECT seq, body FROM events WHERE chat_id = ? AND seq > ? ORDER BY seq")
                .bind(to_sql_int(chat.0))
                .bind(to_sql_int(after.0))
                .fetch_all(&self.pool)
                .await?;
        rows.iter()
            .map(|row| {
                let seq = LedgerSeq(from_sql_int(row.try_get("seq")?));
                let event = serde_json::from_str(row.try_get("body")?)?;
                Ok((seq, event))
            })
            .collect()
    }

    /// 사용량 원값을 그대로 쓴다. `None`인 칸은 NULL. 범위(`UsageScope`), 에이전트, subagent, 모델도 함께 쓴다.
    /// `ThreadCumulative`에서 직전 누적보다 작거나 중간 보고가 빠졌으면 `spans_turns`를 참으로 기록한다.
    /// 중간 보고 누락은 같은 session에서 직전 누적 보고의 실행과 이번 실행 사이에 보고 없는 실행이 있는 경우다.
    ///
    /// # Errors
    /// 없는 실행이면 `NotFound`.
    pub async fn record_usage(
        &self,
        run: RunId,
        session: SessionId,
        report: &UsageReport,
    ) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await?;
        let chat: Option<i64> = sqlx::query_scalar("SELECT chat_id FROM runs WHERE id = ?")
            .bind(to_sql_int(run.0))
            .fetch_optional(&mut *tx)
            .await?;
        let chat = chat.ok_or_else(|| not_found(format!("run {}", run.0)))?;
        let spans_turns = match report.scope {
            UsageScope::ThreadCumulative => spans_turns(&mut tx, run, session, report).await?,
            UsageScope::MainTurn | UsageScope::TreeTotal => false,
        };
        sqlx::query(
            "INSERT INTO usage (chat_id, run_id, session_id, body, scope, agent_id, subagent, model, \
             input_tokens, cache_read_tokens, cache_write_tokens, output_tokens, reasoning_tokens, \
             spans_turns, at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(chat)
        .bind(to_sql_int(run.0))
        .bind(to_sql_int(session.0))
        .bind(serde_json::to_string(report)?)
        .bind(enum_text(&report.scope)?)
        .bind(to_sql_int(report.agent.0))
        .bind(report.subagent.as_ref().map(|id| id.0.clone()))
        .bind(&report.model)
        .bind(report.input.map(to_sql_int))
        .bind(report.cache_read.map(to_sql_int))
        .bind(report.cache_write.map(to_sql_int))
        .bind(report.output.map(to_sql_int))
        .bind(report.reasoning.map(to_sql_int))
        .bind(spans_turns)
        .bind(to_millis(SystemTime::now()))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// `/usage` 범위의 원값 행. `Chat`이면 `chat`이 있어야 한다. 합계는 호출자가 NULL을 빼고 계산한다.
    /// `Today`는 로컬 시각 오늘 0시부터, `Week`는 로컬 시각 이번 주 월요일 0시부터다(초안, 설계에 없음).
    ///
    /// # Errors
    /// 조회 실패면 `Database`, `Chat`인데 `chat`이 없으면 `NotFound`.
    pub async fn usage_rows(
        &self,
        range: UsageRange,
        chat: Option<ChatId>,
    ) -> Result<Vec<UsageRow>, StoreError> {
        const COLUMNS: &str = "SELECT run_id, session_id, body, at, spans_turns FROM usage";
        let query = match range {
            UsageRange::Chat => {
                let chat = chat.ok_or_else(|| not_found("chat for usage range".to_owned()))?;
                sqlx::query(&format!("{COLUMNS} WHERE chat_id = ? ORDER BY id"))
                    .bind(to_sql_int(chat.0))
                    .fetch_all(&self.pool)
                    .await?
            }
            UsageRange::Today => {
                sqlx::query(&format!(
                    "{COLUMNS} WHERE at >= CAST(strftime('%s', 'now', 'localtime', 'start of day', 'utc') \
                     AS INTEGER) * 1000 ORDER BY id"
                ))
                .fetch_all(&self.pool)
                .await?
            }
            UsageRange::Week => {
                sqlx::query(&format!(
                    "{COLUMNS} WHERE at >= CAST(strftime('%s', 'now', 'localtime', 'start of day', \
                     'weekday 0', '-6 days', 'utc') AS INTEGER) * 1000 ORDER BY id"
                ))
                .fetch_all(&self.pool)
                .await?
            }
            UsageRange::All => {
                sqlx::query(&format!("{COLUMNS} ORDER BY id"))
                    .fetch_all(&self.pool)
                    .await?
            }
        };
        query
            .iter()
            .map(|row| {
                Ok(UsageRow {
                    run: RunId(from_sql_int(row.try_get("run_id")?)),
                    session: SessionId(from_sql_int(row.try_get("session_id")?)),
                    report: serde_json::from_str(row.try_get("body")?)?,
                    at: from_millis(row.try_get("at")?),
                    spans_turns: row.try_get("spans_turns")?,
                })
            })
            .collect()
    }
}

/// 누적 사용량이 여러 턴에 걸치는지. 직전 누적보다 작은 칸이 있거나, 직전 누적 보고 뒤에 보고 없는 실행이 있으면 참.
async fn spans_turns(
    tx: &mut sqlx::SqliteConnection,
    run: RunId,
    session: SessionId,
    report: &UsageReport,
) -> Result<bool, StoreError> {
    let previous = sqlx::query(
        "SELECT run_id, body FROM usage WHERE session_id = ? AND scope = ? AND run_id < ? \
         ORDER BY id DESC LIMIT 1",
    )
    .bind(to_sql_int(session.0))
    .bind(enum_text(&UsageScope::ThreadCumulative)?)
    .bind(to_sql_int(run.0))
    .fetch_optional(&mut *tx)
    .await?;
    let after_run = match &previous {
        Some(row) => row.try_get::<i64, _>("run_id")?,
        None => 0,
    };
    let silent_runs: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM runs WHERE session_id = ? AND id > ? AND id < ?")
            .bind(to_sql_int(session.0))
            .bind(after_run)
            .bind(to_sql_int(run.0))
            .fetch_one(&mut *tx)
            .await?;
    if silent_runs > 0 {
        return Ok(true);
    }
    let Some(row) = previous else {
        return Ok(false);
    };
    let before: UsageReport = serde_json::from_str(row.try_get("body")?)?;
    let pairs = [
        (before.input, report.input),
        (before.cache_read, report.cache_read),
        (before.cache_write, report.cache_write),
        (before.output, report.output),
        (before.reasoning, report.reasoning),
    ];
    Ok(pairs
        .iter()
        .any(|pair| matches!(pair, (Some(old), Some(new)) if new < old)))
}

/// 실행 행을 `RunRecord`로.
fn run_record(row: &SqliteRow) -> Result<RunRecord, StoreError> {
    let end = row
        .try_get::<Option<String>, _>("end_kind")?
        .map(|text| parse_run_end(&text))
        .transpose()?;
    Ok(RunRecord {
        id: RunId(from_sql_int(row.try_get("id")?)),
        input: row
            .try_get::<Option<i64>, _>("input_id")?
            .map(|id| InputId(from_sql_int(id))),
        task: TaskId(from_sql_int(row.try_get("task_id")?)),
        session: SessionId(from_sql_int(row.try_get("session_id")?)),
        effect_scope: parse_enum(row.try_get("effect_scope")?)?,
        started_at: from_millis(row.try_get("started_at")?),
        end,
    })
}

/// 고친 행이 없으면 `NotFound`.
pub(crate) fn ensure_found(rows: u64, what: impl FnOnce() -> String) -> Result<(), StoreError> {
    if rows == 0 {
        return Err(not_found(what()));
    }
    Ok(())
}

/// `NotFound` 오류.
pub(crate) fn not_found(what: String) -> StoreError {
    StoreError::NotFound { what }
}

/// 저장된 칸 값이 알 수 없는 문자열일 때. 이 실행 파일이 쓰지 않은 값이다.
fn unknown_value(column: &str, text: &str) -> StoreError {
    StoreError::Database(sqlx::Error::Decode(
        format!("unknown {column} value: {text}").into(),
    ))
}

fn permission_text(permission: Permission) -> &'static str {
    match permission {
        Permission::ReadOnly => "ReadOnly",
        Permission::Write => "Write",
    }
}

fn parse_permission(text: &str) -> Result<Permission, StoreError> {
    match text {
        "ReadOnly" => Ok(Permission::ReadOnly),
        "Write" => Ok(Permission::Write),
        other => Err(unknown_value("permission", other)),
    }
}

fn role_text(role: AgentRole) -> &'static str {
    match role {
        AgentRole::Main => "Main",
        AgentRole::Sub => "Sub",
    }
}

fn parse_role(text: &str) -> Result<AgentRole, StoreError> {
    match text {
        "Main" => Ok(AgentRole::Main),
        "Sub" => Ok(AgentRole::Sub),
        other => Err(unknown_value("role", other)),
    }
}

fn run_end_text(end: RunEnd) -> &'static str {
    match end {
        RunEnd::Completed => "Completed",
        RunEnd::Failed => "Failed",
        RunEnd::Stopped => "Stopped",
    }
}

fn parse_run_end(text: &str) -> Result<RunEnd, StoreError> {
    match text {
        "Completed" => Ok(RunEnd::Completed),
        "Failed" => Ok(RunEnd::Failed),
        "Stopped" => Ok(RunEnd::Stopped),
        other => Err(unknown_value("run end", other)),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use saturn_protocol::event::TurnOrigin;
    use saturn_protocol::state::SessionState;

    use super::*;
    use crate::store::tests::temp_store;

    /// 채팅 하나, 입력 하나, 열린 session 하나, 실행 하나를 만든다.
    pub(crate) async fn chat_with_run(store: &Store) -> (ChatId, InputId, SessionId, RunId) {
        let chat = store.create_chat(PathBuf::from("/work")).await.unwrap();
        let input = store
            .accept_input(&new_input(chat, "fix the build"))
            .await
            .unwrap();
        let session = SessionId(chat.0 * 100);
        store
            .upsert_session(&session_record(session, chat, SessionState::Open))
            .await
            .unwrap();
        let run = store
            .start_run(&new_run(Some(input), session))
            .await
            .unwrap();
        (chat, input, session, run)
    }

    pub(crate) fn new_input(chat: ChatId, text: &str) -> NewInput {
        NewInput {
            chat,
            text: text.to_owned(),
            settings: SettingsRevision(1),
            permission: Permission::Write,
            workdir: PathBuf::from("/work"),
            pinned_model: None,
            skip_relation: false,
        }
    }

    pub(crate) fn new_run(input: Option<InputId>, session: SessionId) -> NewRun {
        NewRun {
            input,
            task: TaskId(1),
            agent: AgentId(1),
            session,
            provider: Provider::Codex,
            effect_scope: EffectScope::NetworkPossible,
        }
    }

    pub(crate) fn session_record(
        id: SessionId,
        chat: ChatId,
        state: SessionState,
    ) -> SessionRecord {
        SessionRecord {
            id,
            chat,
            agent: AgentId(1),
            role: AgentRole::Main,
            provider: Provider::Claude,
            provider_session: Some(ProviderSessionId("thread-1".to_owned())),
            state,
            delivered: LedgerSeq(0),
            idle_since: None,
        }
    }

    fn cumulative(input: Option<u64>, output: Option<u64>) -> UsageReport {
        UsageReport {
            agent: AgentId(1),
            subagent: None,
            model: None,
            scope: UsageScope::ThreadCumulative,
            input,
            cache_read: None,
            cache_write: None,
            output,
            reasoning: None,
        }
    }

    #[tokio::test]
    async fn accepted_input_is_open_until_final_state() {
        let (_dir, store) = temp_store().await;
        let chat = store.create_chat(PathBuf::from("/work")).await.unwrap();
        let input = store.accept_input(&new_input(chat, "hello")).await.unwrap();

        let open = store.open_inputs().await.unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].0, input);
        assert_eq!(open[0].1.text, "hello");
        assert_eq!(open[0].1.permission, Permission::Write);
        assert_eq!(open[0].2, InputState::Judging);

        store
            .set_input_state(input, InputState::Queued, Some(QueueReason::JudgeOrder))
            .await
            .unwrap();
        assert_eq!(store.open_inputs().await.unwrap()[0].2, InputState::Queued);
        store
            .set_input_state(input, InputState::Cancelled, None)
            .await
            .unwrap();
        assert!(store.open_inputs().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn accept_input_for_missing_chat_writes_nothing() {
        let (_dir, store) = temp_store().await;

        let error = store
            .accept_input(&new_input(ChatId(42), "lost"))
            .await
            .unwrap_err();

        assert!(matches!(error, StoreError::Database(_)));
        assert!(store.open_inputs().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn missing_rows_return_not_found() {
        let (_dir, store) = temp_store().await;

        let errors = [
            store.set_recording(ChatId(9), false).await.unwrap_err(),
            store.chat_layer(ChatId(9)).await.unwrap_err(),
            store
                .set_input_state(InputId(9), InputState::Queued, None)
                .await
                .unwrap_err(),
            store.end_stop(ChatId(9)).await.unwrap_err(),
            store
                .finish_run(RunId(9), RunEnd::Completed)
                .await
                .unwrap_err(),
            store
                .start_run(&new_run(None, SessionId(9)))
                .await
                .unwrap_err(),
        ];

        for error in errors {
            assert!(matches!(error, StoreError::NotFound { .. }), "{error:?}");
        }
    }

    #[tokio::test]
    async fn chat_layer_round_trips() {
        let (_dir, store) = temp_store().await;
        let chat = store.create_chat(PathBuf::from("/work")).await.unwrap();

        assert_eq!(store.chat_layer(chat).await.unwrap(), None);
        store.set_chat_layer(chat, "model = \"x\"").await.unwrap();

        assert_eq!(
            store.chat_layer(chat).await.unwrap().as_deref(),
            Some("model = \"x\"")
        );
    }

    #[tokio::test]
    async fn run_stays_unfinished_until_finish() {
        let (_dir, store) = temp_store().await;
        let (_, input, _, run) = chat_with_run(&store).await;

        let open = store.unfinished_runs().await.unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].id, run);
        assert_eq!(open[0].input, Some(input));
        assert_eq!(open[0].effect_scope, EffectScope::NetworkPossible);
        assert_eq!(open[0].end, None);

        store.finish_run(run, RunEnd::Completed).await.unwrap();
        assert!(store.unfinished_runs().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn provider_wake_run_takes_chat_from_session() {
        let (_dir, store) = temp_store().await;
        let (_, _, session, _) = chat_with_run(&store).await;

        let run = store.start_run(&new_run(None, session)).await.unwrap();

        assert_eq!(store.unfinished_runs().await.unwrap()[1].id, run);
    }

    #[tokio::test]
    async fn unobserved_scope_is_never_reverted() {
        let (_dir, store) = temp_store().await;
        let (_, _, _, run) = chat_with_run(&store).await;

        store
            .set_effect_scope(run, EffectScope::Unobserved)
            .await
            .unwrap();
        store
            .set_effect_scope(run, EffectScope::ProvenByObservation)
            .await
            .unwrap();

        let scope = store.unfinished_runs().await.unwrap()[0].effect_scope;
        assert_eq!(scope, EffectScope::Unobserved);
    }

    #[tokio::test]
    async fn session_upsert_updates_state_and_delivered() {
        let (_dir, store) = temp_store().await;
        let (chat, _, session, _) = chat_with_run(&store).await;
        let mut record = session_record(session, chat, SessionState::ClosedResumable);
        record.delivered = LedgerSeq(7);

        store.upsert_session(&record).await.unwrap();

        let sessions = store.sessions(chat).await.unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].state, SessionState::ClosedResumable);
        assert_eq!(sessions[0].delivered, LedgerSeq(7));
        assert_eq!(sessions[0].role, AgentRole::Main);
        assert_eq!(
            sessions[0].provider_session,
            Some(ProviderSessionId("thread-1".to_owned()))
        );
    }

    #[tokio::test]
    async fn events_get_consecutive_numbers_per_chat() {
        let (_dir, store) = temp_store().await;
        let (chat, _, _, run) = chat_with_run(&store).await;
        let text = ProviderEvent::Text {
            agent: AgentId(1),
            subagent: None,
            text: "hi".to_owned(),
        };
        let done = ProviderEvent::TurnCompleted {
            agent: AgentId(1),
            origin: TurnOrigin::User,
        };

        let first = store.append_event(run, chat, &text).await.unwrap();
        let second = store.append_event(run, chat, &done).await.unwrap();

        assert_eq!((first, second), (LedgerSeq(1), LedgerSeq(2)));
        let since = store.events_since(chat, LedgerSeq(1)).await.unwrap();
        assert_eq!(since, vec![(LedgerSeq(2), done)]);
        let missing = store
            .append_event(RunId(99), chat, &text)
            .await
            .unwrap_err();
        assert!(matches!(missing, StoreError::NotFound { .. }));
    }

    #[tokio::test]
    async fn unreported_usage_stays_null() {
        let (_dir, store) = temp_store().await;
        let (chat, _, session, run) = chat_with_run(&store).await;
        let report = UsageReport {
            scope: UsageScope::MainTurn,
            ..cumulative(Some(10), None)
        };

        store.record_usage(run, session, &report).await.unwrap();

        let output: Option<i64> = sqlx::query_scalar("SELECT output_tokens FROM usage")
            .fetch_one(&store.pool)
            .await
            .unwrap();
        assert_eq!(output, None);
        let rows = store
            .usage_rows(UsageRange::Chat, Some(chat))
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].report, report);
        assert!(!rows[0].spans_turns);
        assert_eq!(
            store
                .usage_rows(UsageRange::Today, None)
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            store
                .usage_rows(UsageRange::Week, None)
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            store.usage_rows(UsageRange::All, None).await.unwrap().len(),
            1
        );
        let no_chat = store.usage_rows(UsageRange::Chat, None).await.unwrap_err();
        assert!(matches!(no_chat, StoreError::NotFound { .. }));
    }

    #[tokio::test]
    async fn cumulative_usage_marks_turn_spans() {
        let (_dir, store) = temp_store().await;
        let (chat, input, session, first) = chat_with_run(&store).await;
        store
            .record_usage(first, session, &cumulative(Some(100), Some(10)))
            .await
            .unwrap();
        let second = store
            .start_run(&new_run(Some(input), session))
            .await
            .unwrap();
        store
            .record_usage(second, session, &cumulative(Some(150), Some(20)))
            .await
            .unwrap();
        let _silent = store
            .start_run(&new_run(Some(input), session))
            .await
            .unwrap();
        let fourth = store
            .start_run(&new_run(Some(input), session))
            .await
            .unwrap();
        store
            .record_usage(fourth, session, &cumulative(Some(300), Some(30)))
            .await
            .unwrap();
        let fifth = store
            .start_run(&new_run(Some(input), session))
            .await
            .unwrap();
        store
            .record_usage(fifth, session, &cumulative(Some(50), Some(40)))
            .await
            .unwrap();

        let spans: Vec<bool> = store
            .usage_rows(UsageRange::Chat, Some(chat))
            .await
            .unwrap()
            .iter()
            .map(|row| row.spans_turns)
            .collect();
        assert_eq!(spans, vec![false, false, true, true]);
    }

    #[tokio::test]
    async fn stop_request_begins_and_ends_once() {
        let (_dir, store) = temp_store().await;
        let chat = store.create_chat(PathBuf::from("/work")).await.unwrap();

        store.begin_stop(chat).await.unwrap();
        store.begin_stop(chat).await.unwrap();
        store.end_stop(chat).await.unwrap();

        assert!(matches!(
            store.end_stop(chat).await.unwrap_err(),
            StoreError::NotFound { .. }
        ));
    }
}
