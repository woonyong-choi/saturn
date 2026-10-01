//! 판단 기록의 결과 신호와 물은 답 저장, 느린 조정에 쓸 `Observation` 목록 만들기.
//! 설계: docs/design/records.md

use saturn_core::judges::Answer;
use saturn_core::judges::calibration::{AskedAnswer, Observation, Signal};
use saturn_protocol::ids::JudgmentId;
use serde::Deserialize;
use sqlx::Row;

use super::records::{not_found, unknown_value};
use super::{Store, StoreError, to_sql_int};

#[derive(Deserialize)]
struct StoredAnswer {
    question: String,
    answer: Answer,
}

#[derive(Deserialize)]
struct StoredThreshold {
    question: String,
    threshold: f64,
}

impl Store {
    /// 관찰 시간이 지난 뒤 한 번만 쓴다. 이미 확정한 판단은 바꾸지 않는다.
    ///
    /// # Errors
    /// 없는 판단이면 `NotFound`.
    pub async fn record_signal(
        &self,
        judgment: JudgmentId,
        signal: Signal,
    ) -> Result<(), StoreError> {
        let done = sqlx::query("UPDATE judgments SET signal = ? WHERE id = ? AND signal IS NULL")
            .bind(signal_text(signal))
            .bind(to_sql_int(judgment.0))
            .execute(&self.pool)
            .await?;
        if done.rows_affected() == 0 {
            self.ensure_judgment(judgment).await?;
        }
        Ok(())
    }

    /// 첫 답만 남긴다. 묻지 않은 판단(`asked_with`가 NULL)에는 쓰지 않는다.
    ///
    /// # Errors
    /// 없는 판단이거나 묻지 않은 판단이면 `NotFound`.
    pub async fn record_asked_answer(
        &self,
        judgment: JudgmentId,
        answer: AskedAnswer,
    ) -> Result<(), StoreError> {
        let done = sqlx::query(
            "UPDATE judgments SET asked_answer = ? \
             WHERE id = ? AND asked_with IS NOT NULL AND asked_answer IS NULL",
        )
        .bind(asked_answer_text(answer))
        .bind(to_sql_int(judgment.0))
        .execute(&self.pool)
        .await?;
        if done.rows_affected() > 0 {
            return Ok(());
        }
        let asked: Option<bool> =
            sqlx::query_scalar("SELECT asked_with IS NOT NULL FROM judgments WHERE id = ?")
                .bind(to_sql_int(judgment.0))
                .fetch_optional(&self.pool)
                .await?;
        match asked {
            Some(true) => Ok(()),
            _ => Err(not_found(format!("asked judgment {}", judgment.0))),
        }
    }

    // cost: time O(n·q), heap O(n·q), stack O(1), io 1
    // vars: n = 신호를 확정한 판단 수, q = 판단 하나의 기준값 질문 수
    // basis: estimate
    /// 결과 신호를 확정한 판단마다 기준값이 있는 `noul` 질문 하나가 `Observation` 하나다. 판단 하나의 신호와 물은 답을 그 판단의 모든 질문에 적용한다. 관찰 중인 판단은 뺀다.
    ///
    /// # Errors
    /// 읽기 실패면 `Database`, 저장된 JSON이나 값이 깨졌으면 `Json`이나 `Database`.
    pub async fn observations(&self) -> Result<Vec<Observation>, StoreError> {
        let rows = sqlx::query(
            "SELECT answers, thresholds, asked_with, signal, asked_answer FROM judgments \
             WHERE signal IS NOT NULL ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut observations = Vec::new();
        for row in &rows {
            let answers: Vec<StoredAnswer> = serde_json::from_str(row.try_get("answers")?)?;
            let thresholds: Vec<StoredThreshold> =
                serde_json::from_str(row.try_get("thresholds")?)?;
            let asked_with: Option<f64> = row.try_get("asked_with")?;
            let signal = parse_signal(row.try_get("signal")?)?;
            let asked_answer = row
                .try_get::<Option<&str>, _>("asked_answer")?
                .map(parse_asked_answer)
                .transpose()?;
            for stored in thresholds {
                let probability = answers.iter().find_map(|answer| match &answer.answer {
                    Answer::Noul(yes) if answer.question == stored.question => Some(*yes),
                    _ => None,
                });
                let Some(probability) = probability else {
                    continue;
                };
                observations.push(Observation {
                    question: stored.question,
                    probability,
                    threshold: stored.threshold,
                    // 묻지 않은 판단은 q를 쓰는 계산에 들어가지 않아 1로 둔다
                    asked_with: asked_with.unwrap_or(1.0),
                    is_asked: asked_with.is_some(),
                    signal,
                    asked_answer,
                });
            }
        }
        Ok(observations)
    }

    async fn ensure_judgment(&self, judgment: JudgmentId) -> Result<(), StoreError> {
        let found: Option<i64> = sqlx::query_scalar("SELECT id FROM judgments WHERE id = ?")
            .bind(to_sql_int(judgment.0))
            .fetch_optional(&self.pool)
            .await?;
        found
            .map(|_| ())
            .ok_or_else(|| not_found(format!("judgment {}", judgment.0)))
    }
}

fn signal_text(signal: Signal) -> &'static str {
    match signal {
        Signal::Wrong => "Wrong",
        Signal::Missed => "Missed",
        Signal::Unconfirmed => "Unconfirmed",
    }
}

fn parse_signal(text: &str) -> Result<Signal, StoreError> {
    match text {
        "Wrong" => Ok(Signal::Wrong),
        "Missed" => Ok(Signal::Missed),
        "Unconfirmed" => Ok(Signal::Unconfirmed),
        other => Err(unknown_value("signal", other)),
    }
}

fn asked_answer_text(answer: AskedAnswer) -> &'static str {
    match answer {
        AskedAnswer::Correct => "Correct",
        AskedAnswer::Wrong => "Wrong",
    }
}

fn parse_asked_answer(text: &str) -> Result<AskedAnswer, StoreError> {
    match text {
        "Correct" => Ok(AskedAnswer::Correct),
        "Wrong" => Ok(AskedAnswer::Wrong),
        other => Err(unknown_value("asked_answer", other)),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use saturn_protocol::ids::ChatId;

    use super::*;
    use crate::store::test_judgment as judgment;
    use crate::store::tests::temp_store;

    async fn stored(store: &Store, asked_with: Option<f64>) -> (ChatId, JudgmentId) {
        let chat = store.create_chat(PathBuf::from("/work")).await.unwrap();
        let mut new = judgment(chat);
        new.asked_with = asked_with;
        let id = store.record_judgment(&new).await.unwrap().unwrap();
        (chat, id)
    }

    #[tokio::test]
    async fn observations_carry_signal_answer_and_q_of_the_judgment() {
        let (_dir, store) = temp_store().await;
        let (_, id) = stored(&store, Some(0.25)).await;
        store
            .record_asked_answer(id, AskedAnswer::Wrong)
            .await
            .unwrap();
        store.record_signal(id, Signal::Wrong).await.unwrap();

        let observations = store.observations().await.unwrap();

        assert_eq!(observations.len(), 1);
        let observation = &observations[0];
        assert_eq!(observation.question, "keep_current");
        assert_eq!(observation.probability, 0.9);
        assert_eq!(observation.threshold, 0.8);
        assert_eq!(observation.asked_with, 0.25);
        assert!(observation.is_asked);
        assert_eq!(observation.signal, Signal::Wrong);
        assert_eq!(observation.asked_answer, Some(AskedAnswer::Wrong));
    }

    #[tokio::test]
    async fn observations_skip_judgments_still_being_observed() {
        let (_dir, store) = temp_store().await;
        stored(&store, None).await;

        let observations = store.observations().await.unwrap();

        assert!(observations.is_empty());
    }

    #[tokio::test]
    async fn observations_of_unasked_judgment_have_no_answer() {
        let (_dir, store) = temp_store().await;
        let (_, id) = stored(&store, None).await;
        store.record_signal(id, Signal::Unconfirmed).await.unwrap();

        let observations = store.observations().await.unwrap();

        assert!(!observations[0].is_asked);
        assert_eq!(observations[0].asked_answer, None);
        assert_eq!(observations[0].signal, Signal::Unconfirmed);
    }

    #[tokio::test]
    async fn observations_skip_questions_without_yes_probability() {
        let (_dir, store) = temp_store().await;
        let chat = store.create_chat(PathBuf::from("/work")).await.unwrap();
        let mut new = judgment(chat);
        new.answers = vec![("keep_current".to_owned(), Answer::Choice(vec![0.5, 0.5]))];
        let id = store.record_judgment(&new).await.unwrap().unwrap();
        store.record_signal(id, Signal::Missed).await.unwrap();

        let observations = store.observations().await.unwrap();

        assert!(observations.is_empty());
    }

    #[tokio::test]
    async fn record_signal_keeps_first_confirmed_value() {
        let (_dir, store) = temp_store().await;
        let (_, id) = stored(&store, None).await;
        store.record_signal(id, Signal::Missed).await.unwrap();

        store.record_signal(id, Signal::Wrong).await.unwrap();

        let observations = store.observations().await.unwrap();
        assert_eq!(observations[0].signal, Signal::Missed);
    }

    #[tokio::test]
    async fn record_signal_for_missing_judgment_returns_not_found() {
        let (_dir, store) = temp_store().await;

        let error = store
            .record_signal(JudgmentId(9), Signal::Wrong)
            .await
            .unwrap_err();

        assert!(matches!(error, StoreError::NotFound { .. }));
    }

    #[tokio::test]
    async fn record_asked_answer_keeps_first_answer() {
        let (_dir, store) = temp_store().await;
        let (_, id) = stored(&store, Some(0.1)).await;
        store.record_signal(id, Signal::Unconfirmed).await.unwrap();
        store
            .record_asked_answer(id, AskedAnswer::Correct)
            .await
            .unwrap();

        store
            .record_asked_answer(id, AskedAnswer::Wrong)
            .await
            .unwrap();

        let observations = store.observations().await.unwrap();
        assert_eq!(observations[0].asked_answer, Some(AskedAnswer::Correct));
    }

    #[tokio::test]
    async fn record_asked_answer_for_unasked_judgment_returns_not_found() {
        let (_dir, store) = temp_store().await;
        let (_, id) = stored(&store, None).await;

        let error = store
            .record_asked_answer(id, AskedAnswer::Wrong)
            .await
            .unwrap_err();

        assert!(matches!(error, StoreError::NotFound { .. }));
    }
}
