//! 실행 경계의 폴더 상태 차이로 수정 파일 목록을 세는 순수 규칙.
//! 설계: docs/design/providers-and-sessions.md#수정-파일-목록
//!
//! 파일과 프로세스는 다루지 않는다. 폴더 상태를 읽어 `Snapshot`으로 만드는 일은 engine이 하고,
//! 여기서는 두 스냅샷의 차이와 수정 주체 붙이기, 글로 쓰기만 한다.

use std::collections::{BTreeMap, BTreeSet};

/// 스냅샷 하나가 담는 파일 수의 상한. 넘으면 거기서 멈추고 부분 스냅샷으로 표시한다. 초안.
pub const MAX_FILES: usize = 20_000;

/// 스냅샷 하나를 찍는 시간의 상한(밀리초). 넘으면 거기서 멈추고 부분 스냅샷으로 표시한다. 초안.
pub const MAX_MILLIS: u64 = 3_000;

/// 해시를 계산하는 파일 크기의 상한. 넘는 파일은 크기와 수정 시각으로만 비교한다. 초안.
pub const MAX_HASH_BYTES: u64 = 8 * 1024 * 1024;

/// 패킷과 확인 입력에 풀어 쓰는 파일 수의 상한. 넘으면 개수만 적는다. 초안.
pub const MAX_LISTED: usize = 50;

/// git 저장소가 아닌 폴더에서 훑지 않는 폴더 이름. 빌드 산출물과 의존성 폴더다. 초안.
/// git 저장소에서는 `.gitignore`가 무시 목록이라 쓰지 않는다.
pub const IGNORED_NAMES: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "target",
    "node_modules",
    "build",
    "dist",
    "out",
    ".next",
    ".nuxt",
    ".gradle",
    ".idea",
    ".venv",
    "venv",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".cache",
    ".DS_Store",
];

/// 훑지 않는 이름이면 참.
#[must_use]
pub fn is_ignored_name(name: &str) -> bool {
    IGNORED_NAMES.contains(&name)
}

/// `git status --porcelain`이 파일을 어떻게 보고했는지.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitCode {
    /// 추적하지 않던 새 파일이거나 새로 더한 파일.
    New,
    /// 추적 중인 파일의 내용이나 모드가 바뀜.
    Changed,
    /// 지워짐.
    Gone,
}

/// 파일 하나의 상태. git 저장소에서는 `git`과 `digest`로, 아니면 `size`와 `modified_ms`로 비교한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileState {
    pub size: u64,
    /// unix 밀리초.
    pub modified_ms: i64,
    /// 내용 해시. 상한보다 큰 파일이나 지워진 파일은 `None`.
    pub digest: Option<String>,
    /// git 저장소 파일이면 보고된 상태, 아니면 `None`.
    pub git: Option<GitCode>,
}

impl FileState {
    /// 같은 상태인지 비교하는 값. 해시가 있으면 내용만, 없으면 크기와 수정 시각까지 본다.
    fn same_as(&self, other: &Self) -> bool {
        if self.git == Some(GitCode::Gone) || other.git == Some(GitCode::Gone) {
            return self.git == other.git;
        }
        match (&self.digest, &other.digest) {
            (Some(left), Some(right)) => left == right,
            _ => self.size == other.size && self.modified_ms == other.modified_ms,
        }
    }
}

/// 한 시점의 폴더 상태. 키는 절대 경로다.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub files: BTreeMap<String, FileState>,
    /// git 저장소의 `HEAD` 커밋. 키는 저장소 최상위 폴더. 커밋으로 사라진 수정을 찾는 데 쓴다.
    pub heads: BTreeMap<String, String>,
    /// 상한(파일 수, 시간)에 걸려 일부만 담았다.
    pub is_partial: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub path: String,
    pub kind: ChangeKind,
    /// 이 파일을 고쳤다고 provider 이벤트가 알린 에이전트. 비면 이벤트에 없는 수정(셸 명령, 자식 프로세스)이다.
    pub actors: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChangeSet {
    /// 경로 순서.
    pub files: Vec<FileChange>,
    /// 어느 한쪽 스냅샷이 부분이라 목록이 모자랄 수 있다.
    pub is_partial: bool,
}

/// provider 이벤트가 알린 파일 수정 한 건. `path`는 절대 경로다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub path: String,
    pub actor: String,
}

// cost: time O(n + m log m), heap O(m), stack O(1)
// vars: n = 두 스냅샷의 파일 수, m = 바뀐 파일 수
// basis: estimate
/// 두 스냅샷의 차이. `committed`는 그 사이 새 커밋에 들어간 파일(절대 경로)로, git 상태가 이미 깨끗해져도 목록에 넣는다.
/// 한쪽이 부분 스냅샷이면 git 밖 폴더에서 한쪽에만 있는 파일은 새로 생겼는지 훑지 못한 것인지 알 수 없어 뺀다.
#[must_use]
pub fn diff(before: &Snapshot, after: &Snapshot, committed: &[String]) -> ChangeSet {
    let is_partial = before.is_partial || after.is_partial;
    let paths: BTreeSet<&String> = before.files.keys().chain(after.files.keys()).collect();
    let mut found: BTreeMap<String, ChangeKind> = BTreeMap::new();
    for path in paths {
        let kind = match (before.files.get(path), after.files.get(path)) {
            (Some(old), Some(new)) if old.same_as(new) => None,
            (Some(old), Some(new)) => Some(match (old.git, new.git) {
                (_, Some(GitCode::Gone)) => ChangeKind::Deleted,
                (Some(GitCode::Gone), _) => ChangeKind::Added,
                _ => ChangeKind::Modified,
            }),
            (None, Some(new)) => match new.git {
                None if is_partial => None,
                None | Some(GitCode::New) => Some(ChangeKind::Added),
                Some(GitCode::Gone) => Some(ChangeKind::Deleted),
                Some(GitCode::Changed) => Some(ChangeKind::Modified),
            },
            (Some(old), None) => match old.git {
                None if is_partial => None,
                None => Some(ChangeKind::Deleted),
                Some(_) => Some(ChangeKind::Modified),
            },
            (None, None) => None,
        };
        if let Some(kind) = kind {
            found.insert(path.clone(), kind);
        }
    }
    for path in committed {
        found.entry(path.clone()).or_insert(ChangeKind::Modified);
    }
    ChangeSet {
        files: found
            .into_iter()
            .map(|(path, kind)| FileChange {
                path,
                kind,
                actors: Vec::new(),
            })
            .collect(),
        is_partial,
    }
}

/// 이벤트가 알린 수정 주체를 바뀐 파일에 붙인다. 이벤트에 없는 파일은 주체를 비워 둔다.
/// 파일 목록은 바꾸지 않는다. 이벤트가 알렸어도 폴더 상태가 같으면 목록에 없다.
pub fn attribute(changes: &mut ChangeSet, edits: &[Edit]) {
    for change in &mut changes.files {
        for edit in edits.iter().filter(|edit| edit.path == change.path) {
            if !change.actors.contains(&edit.actor) {
                change.actors.push(edit.actor.clone());
            }
        }
    }
}

/// `git status --porcelain -z` 출력의 한 항목.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PorcelainEntry {
    /// 저장소 최상위 기준 경로.
    pub path: String,
    pub code: GitCode,
}

// cost: time O(L), heap O(n), stack O(1)
// vars: L = 출력 글자 수, n = 항목 수
// basis: estimate
/// 이름이 바뀐 파일은 새 경로를 `Changed`로, 옛 경로를 `Gone`으로 돌려준다. 무시된 파일(`!!`)은 뺀다.
#[must_use]
pub fn parse_porcelain(raw: &str) -> Vec<PorcelainEntry> {
    let mut entries = Vec::new();
    let mut fields = raw.split('\0').filter(|field| !field.is_empty());
    while let Some(field) = fields.next() {
        let mut chars = field.chars();
        let (Some(index), Some(tree), Some(' ')) = (chars.next(), chars.next(), chars.next())
        else {
            continue;
        };
        let path = chars.as_str().to_owned();
        if (index, tree) == ('!', '!') {
            continue;
        }
        if index == 'R' || index == 'C' || tree == 'R' || tree == 'C' {
            if let Some(old) = fields.next()
                && (index == 'R' || tree == 'R')
            {
                entries.push(PorcelainEntry {
                    path: old.to_owned(),
                    code: GitCode::Gone,
                });
            }
            entries.push(PorcelainEntry {
                path,
                code: GitCode::Changed,
            });
            continue;
        }
        let code = match (index, tree) {
            ('?', '?') | ('A', _) => GitCode::New,
            ('D', _) | (_, 'D') => GitCode::Gone,
            _ => GitCode::Changed,
        };
        entries.push(PorcelainEntry { path, code });
    }
    entries
}

/// 패킷과 확인 입력에 쓰는 글. 처음 `MAX_LISTED`개만 풀어 쓰고 나머지는 개수만 적는다.
/// 부분 목록이면 모자랄 수 있다고 밝힌다. 바뀐 파일이 없으면 `None`.
#[must_use]
pub fn describe(changes: &ChangeSet) -> Option<String> {
    if changes.files.is_empty() {
        return None;
    }
    let listed: Vec<String> = changes
        .files
        .iter()
        .take(MAX_LISTED)
        .map(describe_change)
        .collect();
    let mut text = listed.join(", ");
    if changes.files.len() > MAX_LISTED {
        text.push_str(&format!(
            ", and {} more files",
            changes.files.len() - MAX_LISTED
        ));
    }
    if changes.is_partial {
        text.push_str(
            " (partial: the folder was too large to scan fully, so more files may have changed)",
        );
    }
    Some(text)
}

fn describe_change(change: &FileChange) -> String {
    let kind = match change.kind {
        ChangeKind::Added => "added",
        ChangeKind::Modified => "modified",
        ChangeKind::Deleted => "deleted",
    };
    if change.actors.is_empty() {
        format!("{} ({kind}, by shell or child process)", change.path)
    } else {
        format!(
            "{} ({kind}, by {})",
            change.path,
            change.actors.join(" and ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(size: u64, modified_ms: i64) -> FileState {
        FileState {
            size,
            modified_ms,
            digest: None,
            git: None,
        }
    }

    fn tracked(digest: &str, git: GitCode) -> FileState {
        FileState {
            size: 1,
            modified_ms: 1,
            digest: Some(digest.to_owned()),
            git: Some(git),
        }
    }

    fn snapshot(files: &[(&str, FileState)]) -> Snapshot {
        Snapshot {
            files: files
                .iter()
                .map(|(path, state)| ((*path).to_owned(), state.clone()))
                .collect(),
            ..Snapshot::default()
        }
    }

    fn kinds(changes: &ChangeSet) -> Vec<(&str, ChangeKind)> {
        changes
            .files
            .iter()
            .map(|change| (change.path.as_str(), change.kind))
            .collect()
    }

    #[test]
    fn plain_folder_compares_size_and_modified_time() {
        let before = snapshot(&[
            ("/w/same", plain(1, 1)),
            ("/w/grown", plain(1, 1)),
            ("/w/touched", plain(1, 1)),
            ("/w/removed", plain(1, 1)),
        ]);
        let after = snapshot(&[
            ("/w/same", plain(1, 1)),
            ("/w/grown", plain(2, 1)),
            ("/w/touched", plain(1, 2)),
            ("/w/created", plain(5, 5)),
        ]);

        let changes = diff(&before, &after, &[]);

        assert_eq!(
            kinds(&changes),
            vec![
                ("/w/created", ChangeKind::Added),
                ("/w/grown", ChangeKind::Modified),
                ("/w/removed", ChangeKind::Deleted),
                ("/w/touched", ChangeKind::Modified),
            ]
        );
        assert!(!changes.is_partial);
    }

    #[test]
    fn git_folder_compares_content_hash_not_status() {
        let before = snapshot(&[
            ("/w/dirty-before", tracked("a", GitCode::Changed)),
            ("/w/edited-again", tracked("a", GitCode::Changed)),
            ("/w/restored", tracked("a", GitCode::Changed)),
        ]);
        let after = snapshot(&[
            ("/w/dirty-before", tracked("a", GitCode::Changed)),
            ("/w/edited-again", tracked("b", GitCode::Changed)),
            ("/w/fresh", tracked("c", GitCode::New)),
            ("/w/dropped", tracked("", GitCode::Gone)),
        ]);

        let changes = diff(&before, &after, &[]);

        assert_eq!(
            kinds(&changes),
            vec![
                ("/w/dropped", ChangeKind::Deleted),
                ("/w/edited-again", ChangeKind::Modified),
                ("/w/fresh", ChangeKind::Added),
                ("/w/restored", ChangeKind::Modified),
            ]
        );
    }

    #[test]
    fn committed_files_are_listed_even_when_status_is_clean() {
        let clean = Snapshot::default();

        let changes = diff(&clean, &clean, &["/w/committed".to_owned()]);

        assert_eq!(
            kinds(&changes),
            vec![("/w/committed", ChangeKind::Modified)]
        );
    }

    #[test]
    fn partial_snapshot_ignores_plain_presence_differences_and_marks_the_list() {
        let before = snapshot(&[("/w/a", plain(1, 1))]);
        let mut after = snapshot(&[("/w/a", plain(2, 1)), ("/w/b", plain(1, 1))]);
        after.is_partial = true;

        let changes = diff(&before, &after, &[]);

        assert_eq!(kinds(&changes), vec![("/w/a", ChangeKind::Modified)]);
        assert!(changes.is_partial);
    }

    #[test]
    fn attribute_names_the_editor_only_for_files_events_reported() {
        let before = Snapshot::default();
        let after = snapshot(&[("/w/a", plain(1, 1)), ("/w/b", plain(1, 1))]);
        let mut changes = diff(&before, &after, &[]);

        attribute(
            &mut changes,
            &[
                Edit {
                    path: "/w/a".to_owned(),
                    actor: "agent 1".to_owned(),
                },
                Edit {
                    path: "/w/never-changed".to_owned(),
                    actor: "agent 1".to_owned(),
                },
            ],
        );

        assert_eq!(changes.files.len(), 2);
        assert_eq!(changes.files[0].actors, vec!["agent 1".to_owned()]);
        assert!(changes.files[1].actors.is_empty());
    }

    #[test]
    fn porcelain_codes_map_to_new_changed_and_gone() {
        let raw = "?? new.txt\0 M edited.rs\0A  staged.rs\0 D removed.rs\0!! target/x\0R  moved.rs\0old.rs\0";

        let entries = parse_porcelain(raw);

        let seen: Vec<(&str, GitCode)> = entries
            .iter()
            .map(|entry| (entry.path.as_str(), entry.code))
            .collect();
        assert_eq!(
            seen,
            vec![
                ("new.txt", GitCode::New),
                ("edited.rs", GitCode::Changed),
                ("staged.rs", GitCode::New),
                ("removed.rs", GitCode::Gone),
                ("old.rs", GitCode::Gone),
                ("moved.rs", GitCode::Changed),
            ]
        );
    }

    #[test]
    fn describe_lists_at_most_the_limit_and_flags_partial() {
        let files = (0..MAX_LISTED + 3)
            .map(|index| FileChange {
                path: format!("/w/{index:03}"),
                kind: ChangeKind::Modified,
                actors: Vec::new(),
            })
            .collect();
        let changes = ChangeSet {
            files,
            is_partial: true,
        };

        let text = describe(&changes).unwrap();

        assert!(text.contains("/w/000 (modified, by shell or child process)"));
        assert!(!text.contains("/w/050"));
        assert!(text.contains("and 3 more files"));
        assert!(text.contains("partial"));
        assert_eq!(describe(&ChangeSet::default()), None);
    }

    #[test]
    fn build_output_names_are_ignored() {
        assert!(is_ignored_name("target"));
        assert!(is_ignored_name("node_modules"));
        assert!(!is_ignored_name("src"));
    }
}
