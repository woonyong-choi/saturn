//! 항상 허용: 사용자가 허가 창에서 고른 허용을 작업 폴더 단위로 저장한다. provider 설정 파일은 쓰지 않는다.
//! 설계: docs/design/permissions.md#항상-허용-저장

use std::path::Path;
use std::time::SystemTime;

use saturn_core::permission::{Rule, Verdict, parse_tool, tool_name};

use super::{Store, StoreError, to_millis};

impl Store {
    // cost: time O(log n), heap O(1), stack O(1), io 1
    // vars: n = 저장한 항상 허용 수
    // basis: estimate
    /// 같은 폴더, 도구, 패턴이 이미 있으면 그대로 둔다.
    ///
    /// # Errors
    /// 쓰기 실패면 `Database`.
    pub(crate) async fn add_permission_allow(
        &self,
        workdir: &Path,
        rule: &Rule,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT OR IGNORE INTO permission_allows (workdir, tool, pattern, created_at) \
             VALUES (?, ?, ?, ?)",
        )
        .bind(workdir.to_string_lossy().into_owned())
        .bind(tool_name(rule.tool))
        .bind(&rule.pattern)
        .bind(to_millis(SystemTime::now()))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // cost: time O(n), heap O(n), stack O(1), alloc n, io 1
    // vars: n = 그 폴더의 항상 허용 수
    // basis: estimate
    /// 저장 순서대로 `Allow` 규칙을 돌려준다. 알 수 없는 도구 이름의 행은 건너뛴다.
    ///
    /// # Errors
    /// 읽기 실패면 `Database`.
    pub(crate) async fn permission_allows(&self, workdir: &Path) -> Result<Vec<Rule>, StoreError> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT tool, pattern FROM permission_allows WHERE workdir = ? ORDER BY id",
        )
        .bind(workdir.to_string_lossy().into_owned())
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .filter_map(|(tool, pattern)| {
                Some(Rule {
                    tool: parse_tool(&tool)?,
                    pattern,
                    verdict: Verdict::Allow,
                })
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use saturn_core::permission::PermissionTool;

    use super::*;
    use crate::store::tests::temp_store;

    fn allow(tool: PermissionTool, pattern: &str) -> Rule {
        Rule {
            tool,
            pattern: pattern.to_owned(),
            verdict: Verdict::Allow,
        }
    }

    #[tokio::test]
    async fn allows_are_kept_per_workdir_in_saved_order() {
        let (_dir, store) = temp_store().await;
        let work = PathBuf::from("/work");
        let other = PathBuf::from("/other");

        store
            .add_permission_allow(&work, &allow(PermissionTool::Shell, "cargo test"))
            .await
            .unwrap();
        store
            .add_permission_allow(&work, &allow(PermissionTool::Edit, "/outside/a.rs"))
            .await
            .unwrap();
        store
            .add_permission_allow(&other, &allow(PermissionTool::Shell, "make"))
            .await
            .unwrap();

        assert_eq!(
            store.permission_allows(&work).await.unwrap(),
            vec![
                allow(PermissionTool::Shell, "cargo test"),
                allow(PermissionTool::Edit, "/outside/a.rs"),
            ]
        );
        assert_eq!(store.permission_allows(&other).await.unwrap().len(), 1);
        assert!(
            store
                .permission_allows(Path::new("/none"))
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn saving_the_same_allow_twice_keeps_one_row() {
        let (_dir, store) = temp_store().await;
        let work = PathBuf::from("/work");
        let rule = allow(PermissionTool::Shell, "cargo test");

        store.add_permission_allow(&work, &rule).await.unwrap();
        store.add_permission_allow(&work, &rule).await.unwrap();

        assert_eq!(store.permission_allows(&work).await.unwrap().len(), 1);
    }
}
