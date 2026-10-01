//! 판단 기록 저장, 전용 정리, JSONL 내보내기. 일반 정리(`prune`)는 판단 기록을 지우지 않는다.
//! 설계: docs/design/records.md

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::time::{Duration, SystemTime};

use saturn_core::judges::{Answer, Method, QuestionSetId};
use saturn_protocol::ids::{ChatId, InputId, JudgmentId, SettingsRevision};
use serde_json::{Value, json};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use super::records::not_found;
use super::{Store, StoreError, enum_text, from_sql_int, to_millis, to_sql_int};
use crate::secrets::Masked;

/// 판단 원문이 들어 있어 소유자만 읽고 쓴다. 초안 값.
const EXPORT_FILE_MODE: u32 = 0o600;

pub use saturn_core::judges::JudgmentOutcome;

#[derive(Debug, Clone)]
pub struct NewJudgment {
    /// `/record off`인지 이것으로 본다.
    pub chat: ChatId,
    /// 입력과 무관한 호출(`compact`, `loop` 등)은 `None`.
    pub input: Option<InputId>,
    pub method: Method,
    /// 중립 이름. 예: `jev`, `saturn-local`.
    pub judge: String,
    /// 요청 모델과 응답이 보고한 실제 모델.
    pub model: (String, Option<String>),
    /// 버전이 바뀐 뒤 옛 기록을 다시 해석하는 데 쓴다.
    pub question_sets: Vec<QuestionSetId>,
    pub settings: SettingsRevision,
    /// 비밀값을 가린 요청 본문.
    pub sent: Masked,
    /// 비밀값을 가린 응답 본문.
    pub received: Option<Masked>,
    pub answers: Vec<(String, Answer)>,
    /// 대체 규칙을 적용한 질문 id와 사유.
    pub fallbacks: Vec<(String, String)>,
    /// 입력, 출력 토큰. 보고되지 않았거나 `CostUnknown`이면 `None`(NULL).
    pub tokens: Option<(u64, u64)>,
    pub started_at: SystemTime,
    /// 응답이 없으면 포기할 때까지의 시간.
    pub elapsed: Duration,
    pub outcome: JudgmentOutcome,
    /// 모델, 보정값, 질문별 목표 틀림 비율 묶음. 기준값 조정 계산을 다시 하는 데 쓴다.
    pub judge_version: String,
    pub thresholds: Vec<(String, f64)>,
    /// 피드백 질문을 한 확률 q. 계산 때 1/q로 가중한다.
    pub asked_with: Option<f64>,
}

/// `yes`가 거짓이면 미리보기만 한다.
#[derive(Debug, Clone)]
pub struct JudgmentPruneRequest {
    /// `None`이면 전부.
    pub before: Option<SystemTime>,
    /// `None`이면 모든 채팅.
    pub chat: Option<ChatId>,
    pub yes: bool,
}

impl Store {
    /// 채팅이 `/record off`면 쓰지 않고 `None`을 돌려준다.
    pub async fn record_judgment(
        &self,
        judgment: &NewJudgment,
    ) -> Result<Option<JudgmentId>, StoreError> {
        let mut tx = self.pool.begin().await?;
        let recording: Option<bool> =
            sqlx::query_scalar("SELECT recording FROM chats WHERE id = ?")
                .bind(to_sql_int(judgment.chat.0))
                .fetch_optional(&mut *tx)
                .await?;
        let recording = recording.ok_or_else(|| not_found(format!("chat {}", judgment.chat.0)))?;
        if !recording {
            return Ok(None);
        }
        let question_sets: Vec<String> = judgment
            .question_sets
            .iter()
            .map(question_set_label)
            .collect();
        let answers: Vec<Value> = judgment
            .answers
            .iter()
            .map(|(question, answer)| json!({ "question": question, "answer": answer }))
            .collect();
        let fallbacks: Vec<Value> = judgment
            .fallbacks
            .iter()
            .map(|(question, reason)| json!({ "question": question, "reason": reason }))
            .collect();
        let thresholds: Vec<Value> = judgment
            .thresholds
            .iter()
            .map(|(question, value)| json!({ "question": question, "threshold": value }))
            .collect();
        let elapsed_ms = i64::try_from(judgment.elapsed.as_millis()).unwrap_or(i64::MAX);
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO judgments (chat_id, input_id, method, judge, model, reported_model, question_sets, \
             settings_revision, sent, received, answers, fallbacks, input_tokens, output_tokens, started_at, \
             elapsed_ms, outcome, judge_version, thresholds, asked_with) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(to_sql_int(judgment.chat.0))
        .bind(judgment.input.map(|input| to_sql_int(input.0)))
        .bind(method_text(judgment.method))
        .bind(&judgment.judge)
        .bind(&judgment.model.0)
        .bind(&judgment.model.1)
        .bind(serde_json::to_string(&question_sets)?)
        .bind(to_sql_int(judgment.settings.0))
        .bind(judgment.sent.as_str())
        .bind(judgment.received.as_ref().map(Masked::as_str))
        .bind(serde_json::to_string(&answers)?)
        .bind(serde_json::to_string(&fallbacks)?)
        .bind(judgment.tokens.map(|(input, _)| to_sql_int(input)))
        .bind(judgment.tokens.map(|(_, output)| to_sql_int(output)))
        .bind(to_millis(judgment.started_at))
        .bind(elapsed_ms)
        .bind(enum_text(&judgment.outcome)?)
        .bind(&judgment.judge_version)
        .bind(serde_json::to_string(&thresholds)?)
        .bind(judgment.asked_with)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Some(JudgmentId(from_sql_int(id))))
    }

    /// `yes`가 거짓이면 지울 건수만 돌려준다. 채팅 삭제가 아니라 삭제 흔적은 남기지 않는다.
    pub async fn prune_judgments(&self, request: &JudgmentPruneRequest) -> Result<u64, StoreError> {
        const FILTER: &str =
            "WHERE (?1 IS NULL OR started_at < ?1) AND (?2 IS NULL OR chat_id = ?2)";
        let before = request.before.map(to_millis);
        let chat = request.chat.map(|chat| to_sql_int(chat.0));
        if !request.yes {
            let count: i64 =
                sqlx::query_scalar(&format!("SELECT COUNT(*) FROM judgments {FILTER}"))
                    .bind(before)
                    .bind(chat)
                    .fetch_one(&self.pool)
                    .await?;
            return Ok(from_sql_int(count));
        }
        let done = sqlx::query(&format!("DELETE FROM judgments {FILTER}"))
            .bind(before)
            .bind(chat)
            .execute(&self.pool)
            .await?;
        if done.rows_affected() > 0 {
            self.compact_file().await?;
        }
        Ok(done.rows_affected())
    }

    /// 이미 있는 파일에는 쓰지 않는다. 필드 목록과 권한 0600은 초안이다.
    ///
    /// # Errors
    /// 파일 쓰기 실패면 `Export`, 직렬화 실패면 `Json`.
    pub async fn export_judgments(&self, path: &Path) -> Result<u64, StoreError> {
        let rows = sqlx::query("SELECT * FROM judgments ORDER BY id")
            .fetch_all(&self.pool)
            .await?;
        let export_error = |source| StoreError::Export {
            path: path.to_path_buf(),
            source,
        };
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(EXPORT_FILE_MODE)
            .open(path)
            .map_err(export_error)?;
        let mut out = std::io::BufWriter::new(file);
        for row in &rows {
            let line = serde_json::to_string(&export_line(row)?)?;
            writeln!(out, "{line}").map_err(export_error)?;
        }
        out.flush().map_err(export_error)?;
        Ok(rows.len() as u64)
    }
}

/// 저장한 JSON 칸은 문자열이 아니라 값으로 펼친다.
fn export_line(row: &SqliteRow) -> Result<Value, StoreError> {
    let parse = |column: &str| -> Result<Value, StoreError> {
        Ok(serde_json::from_str(row.try_get(column)?)?)
    };
    let input_tokens: Option<i64> = row.try_get("input_tokens")?;
    let output_tokens: Option<i64> = row.try_get("output_tokens")?;
    let tokens = match (input_tokens, output_tokens) {
        (Some(input), Some(output)) => json!({ "input": input, "output": output }),
        _ => Value::Null,
    };
    Ok(json!({
        "id": row.try_get::<i64, _>("id")?,
        "chat": row.try_get::<i64, _>("chat_id")?,
        "input": row.try_get::<Option<i64>, _>("input_id")?,
        "method": row.try_get::<String, _>("method")?,
        "judge": row.try_get::<String, _>("judge")?,
        "model": {
            "requested": row.try_get::<String, _>("model")?,
            "reported": row.try_get::<Option<String>, _>("reported_model")?,
        },
        "question_sets": parse("question_sets")?,
        "settings": row.try_get::<i64, _>("settings_revision")?,
        "sent": row.try_get::<String, _>("sent")?,
        "received": row.try_get::<Option<String>, _>("received")?,
        "answers": parse("answers")?,
        "fallbacks": parse("fallbacks")?,
        "tokens": tokens,
        "started_at": row.try_get::<i64, _>("started_at")?,
        "elapsed_ms": row.try_get::<i64, _>("elapsed_ms")?,
        "outcome": row.try_get::<String, _>("outcome")?,
    }))
}

/// `route@3.1` 형식.
fn question_set_label(set: &QuestionSetId) -> String {
    format!("{}@{}.{}", set.name, set.major, set.minor)
}

fn method_text(method: Method) -> &'static str {
    match method {
        Method::Jev => "jev",
        Method::Saturn => "saturn",
        Method::Collect => "collect",
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    use super::*;
    use crate::store::records::tests::chat_with_run;
    use crate::store::tests::temp_store;
    use crate::store::{PruneOutcome, PruneRequest, PruneScope, RunEnd};

    fn judgment(chat: ChatId) -> NewJudgment {
        NewJudgment {
            chat,
            input: None,
            method: Method::Jev,
            judge: "jev".to_owned(),
            model: ("judge-1".to_owned(), None),
            question_sets: vec![QuestionSetId {
                name: "route".to_owned(),
                major: 3,
                minor: 1,
            }],
            settings: SettingsRevision(1),
            sent: Masked::assume_masked("{\"state\":\"[redacted]\"}"),
            received: None,
            answers: vec![("keep_current".to_owned(), Answer::Noul(0.9))],
            fallbacks: Vec::new(),
            tokens: None,
            started_at: SystemTime::now(),
            elapsed: Duration::from_millis(120),
            outcome: JudgmentOutcome::NoResponse,
            judge_version: "v1".to_owned(),
            thresholds: vec![("keep_current".to_owned(), 0.8)],
            asked_with: None,
        }
    }

    #[tokio::test]
    async fn recording_off_skips_judgment() {
        let (_dir, store) = temp_store().await;
        let chat = store.create_chat(PathBuf::from("/work")).await.unwrap();

        assert!(
            store
                .record_judgment(&judgment(chat))
                .await
                .unwrap()
                .is_some()
        );
        store.set_recording(chat, false).await.unwrap();
        assert!(
            store
                .record_judgment(&judgment(chat))
                .await
                .unwrap()
                .is_none()
        );

        let count = store
            .prune_judgments(&JudgmentPruneRequest {
                before: None,
                chat: None,
                yes: false,
            })
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn chat_prune_keeps_judgments() {
        let (_dir, store) = temp_store().await;
        let (chat, input, _, run) = chat_with_run(&store).await;
        store.record_judgment(&judgment(chat)).await.unwrap();
        store.finish_run(run, RunEnd::Completed).await.unwrap();
        store
            .set_input_state(input, saturn_protocol::state::InputState::Applied, None)
            .await
            .unwrap();
        sqlx::query("DELETE FROM sessions")
            .execute(&store.pool)
            .await
            .unwrap();

        let outcome = store
            .prune(&PruneRequest {
                scope: PruneScope::Chats(vec![chat]),
                yes: true,
            })
            .await
            .unwrap();

        assert!(
            matches!(outcome, PruneOutcome::Deleted { ref plan, .. } if plan.chats == vec![chat])
        );
        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM judgments")
            .fetch_one(&store.pool)
            .await
            .unwrap();
        assert_eq!(left, 1);
    }

    #[tokio::test]
    async fn judgment_prune_previews_then_deletes() {
        let (_dir, store) = temp_store().await;
        let chat = store.create_chat(PathBuf::from("/work")).await.unwrap();
        let other = store.create_chat(PathBuf::from("/work")).await.unwrap();
        store.record_judgment(&judgment(chat)).await.unwrap();
        store.record_judgment(&judgment(other)).await.unwrap();
        let only_chat = |yes| JudgmentPruneRequest {
            before: None,
            chat: Some(chat),
            yes,
        };

        assert_eq!(store.prune_judgments(&only_chat(false)).await.unwrap(), 1);
        assert_eq!(store.prune_judgments(&only_chat(false)).await.unwrap(), 1);
        assert_eq!(store.prune_judgments(&only_chat(true)).await.unwrap(), 1);
        assert_eq!(store.prune_judgments(&only_chat(false)).await.unwrap(), 0);
        let all = JudgmentPruneRequest {
            before: None,
            chat: None,
            yes: false,
        };
        assert_eq!(store.prune_judgments(&all).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn export_writes_one_json_per_line() {
        let (dir, store) = temp_store().await;
        let chat = store.create_chat(PathBuf::from("/work")).await.unwrap();
        store.record_judgment(&judgment(chat)).await.unwrap();
        let mut second = judgment(chat);
        second.tokens = Some((30, 2));
        second.outcome = JudgmentOutcome::Ok;
        store.record_judgment(&second).await.unwrap();
        let path = dir.path().join("judgments.jsonl");

        assert_eq!(store.export_judgments(&path).await.unwrap(), 2);

        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["question_sets"], json!(["route@3.1"]));
        assert_eq!(lines[0]["tokens"], Value::Null);
        assert_eq!(lines[1]["tokens"], json!({ "input": 30, "output": 2 }));
        assert_eq!(lines[1]["outcome"], json!("Ok"));
        assert_eq!(lines[0]["answers"][0]["answer"], json!({ "Noul": 0.9 }));
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, EXPORT_FILE_MODE);
        let again = store.export_judgments(&path).await.unwrap_err();
        assert!(matches!(again, StoreError::Export { .. }));
    }
}
