//! 실행마다 수정 파일 목록: 실행 경계의 폴더 상태 차이로 센 결과를 남긴다.
//! 설계: docs/design/records.md#실행별-수정-파일

use saturn_core::sessions::changes::{ChangeKind, ChangeSet, FileChange};
use saturn_protocol::ids::{ChatId, LedgerSeq, RunId, SessionId};
use sqlx::Row;

use super::{Store, StoreError, from_sql_int, to_sql_int};

const COMPLETE: &str = "complete";
const PARTIAL: &str = "partial";
const ACTOR_SEPARATOR: char = '\n';

/// 한 실행이 바꾼 파일. 기록 번호는 그 실행의 마지막 이벤트 번호다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunChanges {
    pub(crate) run: RunId,
    pub(crate) session: SessionId,
    pub(crate) seq: LedgerSeq,
    pub(crate) at_ms: i64,
    pub(crate) set: ChangeSet,
}

impl Store {
    // cost: time O(n log m), heap O(n), stack O(1), io n + 1
    // vars: n = 바뀐 파일 수, m = 저장한 수정 파일 행 수
    // basis: estimate
    /// 같은 실행의 이전 목록은 바꿔 쓴다. 바뀐 파일이 없어도 측정했다는 표시는 남는다.
    ///
    /// # Errors
    /// 쓰기 실패면 `Database`.
    pub(crate) async fn record_run_changes(
        &self,
        run: RunId,
        chat: ChatId,
        set: &ChangeSet,
    ) -> Result<(), StoreError> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("DELETE FROM run_changes WHERE run_id = ?")
            .bind(to_sql_int(run.0))
            .execute(&mut *transaction)
            .await?;
        for change in &set.files {
            sqlx::query(
                "INSERT INTO run_changes (run_id, chat_id, path, kind, actors) VALUES (?, ?, ?, ?, ?)",
            )
            .bind(to_sql_int(run.0))
            .bind(to_sql_int(chat.0))
            .bind(&change.path)
            .bind(kind_text(change.kind))
            .bind(change.actors.join(&ACTOR_SEPARATOR.to_string()))
            .execute(&mut *transaction)
            .await?;
        }
        sqlx::query("UPDATE runs SET changes_state = ? WHERE id = ?")
            .bind(if set.is_partial { PARTIAL } else { COMPLETE })
            .bind(to_sql_int(run.0))
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    // cost: time O(r + n), heap O(r + n), stack O(1), io 2
    // vars: r = 그 채팅의 측정한 실행 수, n = 그 실행들의 수정 파일 행 수
    // basis: estimate
    /// 측정한 실행의 목록. 바뀐 파일이 없고 부분도 아닌 실행은 뺀다. 실행 순서다.
    ///
    /// # Errors
    /// 읽기 실패면 `Database`.
    pub(crate) async fn run_changes(&self, chat: ChatId) -> Result<Vec<RunChanges>, StoreError> {
        let runs = sqlx::query(
            "SELECT r.id, r.session_id, r.changes_state, COALESCE(r.ended_at, r.started_at) AS at, \
             COALESCE((SELECT MAX(e.seq) FROM events e WHERE e.run_id = r.id), 0) AS last_seq \
             FROM runs r WHERE r.chat_id = ? AND r.changes_state IS NOT NULL ORDER BY r.id",
        )
        .bind(to_sql_int(chat.0))
        .fetch_all(&self.pool)
        .await?;
        let files = sqlx::query(
            "SELECT run_id, path, kind, actors FROM run_changes WHERE chat_id = ? ORDER BY run_id, path",
        )
        .bind(to_sql_int(chat.0))
        .fetch_all(&self.pool)
        .await?;
        let mut found = Vec::new();
        for run in &runs {
            let id = RunId(from_sql_int(run.try_get("id")?));
            let mut set = ChangeSet {
                files: Vec::new(),
                is_partial: run.try_get::<String, _>("changes_state")? == PARTIAL,
            };
            for file in files
                .iter()
                .filter(|file| file.try_get::<i64, _>("run_id").ok() == Some(to_sql_int(id.0)))
            {
                let actors: String = file.try_get("actors")?;
                set.files.push(FileChange {
                    path: file.try_get("path")?,
                    kind: parse_kind(&file.try_get::<String, _>("kind")?),
                    actors: actors
                        .split(ACTOR_SEPARATOR)
                        .filter(|actor| !actor.is_empty())
                        .map(str::to_owned)
                        .collect(),
                });
            }
            if set.files.is_empty() && !set.is_partial {
                continue;
            }
            found.push(RunChanges {
                run: id,
                session: SessionId(from_sql_int(run.try_get("session_id")?)),
                seq: LedgerSeq(from_sql_int(run.try_get("last_seq")?)),
                at_ms: run.try_get("at")?,
                set,
            });
        }
        Ok(found)
    }
}

fn kind_text(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Added => "added",
        ChangeKind::Modified => "modified",
        ChangeKind::Deleted => "deleted",
    }
}

fn parse_kind(text: &str) -> ChangeKind {
    match text {
        "added" => ChangeKind::Added,
        "deleted" => ChangeKind::Deleted,
        _ => ChangeKind::Modified,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::records::tests::chat_with_run;
    use crate::store::tests::temp_store;

    fn change(path: &str, kind: ChangeKind, actors: &[&str]) -> FileChange {
        FileChange {
            path: path.to_owned(),
            kind,
            actors: actors.iter().map(|actor| (*actor).to_owned()).collect(),
        }
    }

    #[tokio::test]
    async fn run_changes_round_trip_with_actors_and_partial_mark() {
        let (_dir, store) = temp_store().await;
        let (chat, _input, session, run) = chat_with_run(&store).await;
        let set = ChangeSet {
            files: vec![
                change("/w/a.rs", ChangeKind::Modified, &["agent 1"]),
                change("/w/b.rs", ChangeKind::Added, &[]),
            ],
            is_partial: true,
        };

        store.record_run_changes(run, chat, &set).await.unwrap();

        let stored = store.run_changes(chat).await.unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!((stored[0].run, stored[0].session), (run, session));
        assert_eq!(stored[0].set, set);
    }

    #[tokio::test]
    async fn recording_again_replaces_the_list_and_empty_runs_are_not_returned() {
        let (_dir, store) = temp_store().await;
        let (chat, _input, _session, run) = chat_with_run(&store).await;
        let first = ChangeSet {
            files: vec![change("/w/a.rs", ChangeKind::Added, &[])],
            is_partial: false,
        };
        store.record_run_changes(run, chat, &first).await.unwrap();

        store
            .record_run_changes(run, chat, &ChangeSet::default())
            .await
            .unwrap();

        assert!(store.run_changes(chat).await.unwrap().is_empty());
    }
}
