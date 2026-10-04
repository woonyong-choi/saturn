//! 작업 폴더 상태 읽기: 실행 경계마다 폴더를 훑어 `Snapshot`을 만든다.
//! 설계: docs/design/providers-and-sessions.md#수정-파일-목록
//!
//! 비교 규칙은 `saturn_core::sessions::changes`가 갖고, 여기서는 파일과 `git`만 읽는다.
//! git 저장소는 `git status --porcelain`이 알린 파일의 해시로, 아니면 수정 시각과 크기로 센다.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant, UNIX_EPOCH};

use saturn_core::sessions::changes::{
    FileState, GitCode, MAX_FILES, MAX_HASH_BYTES, MAX_MILLIS, Snapshot, is_ignored_name,
    parse_porcelain,
};
use sha2::{Digest, Sha256};

/// 스냅샷 하나의 비용 상한. 넘으면 거기서 멈추고 부분 스냅샷으로 표시한다.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Limits {
    pub(crate) max_files: usize,
    pub(crate) max_time: Duration,
    pub(crate) max_hash_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_files: MAX_FILES,
            max_time: Duration::from_millis(MAX_MILLIS),
            max_hash_bytes: MAX_HASH_BYTES,
        }
    }
}

// cost: time O(F), heap O(F), stack O(d), io F
// vars: F = 훑은 파일 수(`max_files` 이하), d = 폴더 깊이
// basis: estimate
/// 폴더들의 지금 상태. 폴더마다 git 저장소인지 보고 방식을 고른다. 시간이나 파일 수 상한에 걸리면 `is_partial`이다.
/// 파일을 읽는 동기 함수라 비동기 흐름에서는 `spawn_blocking`으로 부른다.
pub(crate) fn capture(folders: &[PathBuf], limits: &Limits) -> Snapshot {
    let deadline = Instant::now() + limits.max_time;
    let mut snapshot = Snapshot::default();
    for folder in folders {
        match git_root(folder, deadline) {
            Some(root) => capture_git(folder, &root, limits, deadline, &mut snapshot),
            None => capture_plain(folder, limits, deadline, &mut snapshot),
        }
    }
    snapshot
}

/// `before`를 찍은 뒤 새 커밋에 들어간 파일의 절대 경로. 커밋하면 git 상태가 깨끗해져 상태 차이로는 보이지 않는다.
/// `folders` 밖의 파일은 뺀다.
pub(crate) fn committed_since(
    before: &Snapshot,
    folders: &[PathBuf],
    limits: &Limits,
) -> Vec<String> {
    let deadline = Instant::now() + limits.max_time;
    let mut found = Vec::new();
    for (root, old_head) in &before.heads {
        let root = Path::new(root);
        let Some(head) = git_head(root, deadline) else {
            continue;
        };
        if &head == old_head {
            continue;
        }
        let range = format!("{old_head}..{head}");
        let Some(raw) = git(
            root,
            &["diff", "--name-only", "-z", "--no-renames", &range],
            deadline,
        ) else {
            continue;
        };
        found.extend(
            String::from_utf8_lossy(&raw)
                .split('\0')
                .filter(|name| !name.is_empty())
                .map(|name| root.join(name))
                .filter(|path| folders.iter().any(|folder| path.starts_with(folder)))
                .map(|path| path.to_string_lossy().into_owned()),
        );
    }
    found
}

fn git_root(folder: &Path, deadline: Instant) -> Option<PathBuf> {
    let raw = git(folder, &["rev-parse", "--show-toplevel"], deadline)?;
    let root = String::from_utf8(raw).ok()?;
    let root = root.trim_end_matches('\n');
    (!root.is_empty()).then(|| PathBuf::from(root))
}

fn git_head(root: &Path, deadline: Instant) -> Option<String> {
    let raw = git(root, &["rev-parse", "HEAD"], deadline)?;
    Some(String::from_utf8(raw).ok()?.trim().to_owned())
}

/// 저장소 최상위에서 `folder` 아래의 변경 파일을 읽는다. 무시 목록은 `.gitignore`를 `git`이 적용한다.
fn capture_git(
    folder: &Path,
    root: &Path,
    limits: &Limits,
    deadline: Instant,
    snapshot: &mut Snapshot,
) {
    if let Some(head) = git_head(root, deadline) {
        snapshot
            .heads
            .insert(root.to_string_lossy().into_owned(), head);
    }
    let Some(raw) = git(
        folder,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--",
            ".",
        ],
        deadline,
    ) else {
        snapshot.is_partial = true;
        return;
    };
    for entry in parse_porcelain(&String::from_utf8_lossy(&raw)) {
        if snapshot.files.len() >= limits.max_files || Instant::now() >= deadline {
            snapshot.is_partial = true;
            return;
        }
        let path = root.join(&entry.path);
        let state = match entry.code {
            GitCode::Gone => FileState {
                size: 0,
                modified_ms: 0,
                digest: None,
                git: Some(GitCode::Gone),
            },
            code => match file_state(&path, Some(limits.max_hash_bytes), Some(code)) {
                Some(state) => state,
                None => continue,
            },
        };
        snapshot
            .files
            .insert(path.to_string_lossy().into_owned(), state);
    }
}

/// 이름순으로 내려가며 훑는다. 링크는 따라가지 않고, 무시 이름의 폴더와 파일은 건너뛴다.
fn capture_plain(folder: &Path, limits: &Limits, deadline: Instant, snapshot: &mut Snapshot) {
    let mut pending = vec![folder.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<_> = read.filter_map(Result::ok).collect();
        entries.sort_by_key(std::fs::DirEntry::file_name);
        let mut subdirs = Vec::new();
        for entry in entries {
            if snapshot.files.len() >= limits.max_files || Instant::now() >= deadline {
                snapshot.is_partial = true;
                return;
            }
            if is_ignored_name(&entry.file_name().to_string_lossy()) {
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                subdirs.push(entry.path());
            } else if kind.is_file()
                && let Some(state) = file_state(&entry.path(), None, None)
            {
                snapshot
                    .files
                    .insert(entry.path().to_string_lossy().into_owned(), state);
            }
        }
        pending.extend(subdirs.into_iter().rev());
    }
}

/// 크기와 수정 시각, 그리고 `hash_limit` 이하인 파일의 해시(`None`이면 해시하지 않는다). 읽을 수 없는 파일은 `None`.
fn file_state(path: &Path, hash_limit: Option<u64>, git: Option<GitCode>) -> Option<FileState> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    let modified_ms = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(0));
    let digest = hash_limit
        .filter(|limit| meta.is_file() && meta.len() <= *limit)
        .and_then(|_| hash_file(path));
    Some(FileState {
        size: meta.len(),
        modified_ms,
        digest,
        git,
    })
}

fn hash_file(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).ok()?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Some(
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    )
}

/// `git`을 `dir`에서 실행해 출력을 받는다. 마감이 지나면 이 자식만 죽이고 `None`이다. 실패해도 `None`이다.
/// 잠금을 잡거나 묻지 않게 하고, 출력은 읽는 스레드가 비워 파이프가 막히지 않게 한다.
fn git(dir: &Path, args: &[&str], deadline: Instant) -> Option<Vec<u8>> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut out = Vec::new();
        let read = stdout.read_to_end(&mut out);
        let _ = sender.send(read.map(|_| out));
    });
    let wait = deadline.saturating_duration_since(Instant::now());
    match receiver.recv_timeout(wait) {
        Ok(Ok(out)) => child
            .wait()
            .ok()
            .filter(std::process::ExitStatus::success)
            .map(|_| out),
        _ => {
            let _ = child.kill();
            let _ = child.wait();
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use saturn_core::sessions::changes::{ChangeKind, diff};

    use super::*;

    fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        path
    }

    fn run_git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    fn changed(before: &Snapshot, after: &Snapshot, dir: &Path) -> Vec<(String, ChangeKind)> {
        let committed = committed_since(before, &[dir.to_path_buf()], &Limits::default());
        diff(before, after, &committed)
            .files
            .into_iter()
            .map(|change| {
                let name = Path::new(&change.path)
                    .strip_prefix(dir)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                (name, change.kind)
            })
            .collect()
    }

    fn folder() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().canonicalize().unwrap();
        (dir, path)
    }

    #[test]
    fn plain_folder_sees_files_changed_without_any_event() {
        let (_guard, dir) = folder();
        write(&dir, "keep.txt", "same");
        write(&dir, "edit.txt", "one");
        write(&dir, "gone.txt", "bye");
        write(&dir, "node_modules/lib/index.js", "ignored");
        let limits = Limits::default();
        let before = capture(std::slice::from_ref(&dir), &limits);

        write(&dir, "edit.txt", "one two");
        std::fs::remove_file(dir.join("gone.txt")).unwrap();
        write(&dir, "sub/new.txt", "hello");
        write(&dir, "node_modules/lib/index.js", "ignored but changed");
        write(&dir, "target/out.bin", "build output");
        let after = capture(std::slice::from_ref(&dir), &limits);

        assert_eq!(
            changed(&before, &after, &dir),
            vec![
                ("edit.txt".to_owned(), ChangeKind::Modified),
                ("gone.txt".to_owned(), ChangeKind::Deleted),
                ("sub/new.txt".to_owned(), ChangeKind::Added),
            ]
        );
        assert!(!after.is_partial);
    }

    #[test]
    fn git_folder_sees_shell_edits_by_content_and_respects_gitignore() {
        let (_guard, dir) = folder();
        run_git(&dir, &["init", "-q"]);
        write(&dir, ".gitignore", "*.log\nbuild/\n");
        write(&dir, "tracked.txt", "v1");
        write(&dir, "dirty.txt", "v1");
        run_git(&dir, &["add", "."]);
        run_git(&dir, &["commit", "-q", "-m", "init"]);
        write(&dir, "dirty.txt", "v2 by the user before the run");
        let limits = Limits::default();
        let before = capture(std::slice::from_ref(&dir), &limits);

        write(&dir, "tracked.txt", "v2");
        write(&dir, "dirty.txt", "v3 edited again during the run");
        write(&dir, "fresh.txt", "new");
        write(&dir, "debug.log", "ignored");
        write(&dir, "build/out.bin", "ignored");
        let after = capture(std::slice::from_ref(&dir), &limits);

        assert_eq!(
            changed(&before, &after, &dir),
            vec![
                ("dirty.txt".to_owned(), ChangeKind::Modified),
                ("fresh.txt".to_owned(), ChangeKind::Added),
                ("tracked.txt".to_owned(), ChangeKind::Modified),
            ]
        );
    }

    #[test]
    fn git_folder_keeps_files_the_run_committed() {
        let (_guard, dir) = folder();
        run_git(&dir, &["init", "-q"]);
        write(&dir, "a.txt", "v1");
        run_git(&dir, &["add", "."]);
        run_git(&dir, &["commit", "-q", "-m", "init"]);
        let before = capture(std::slice::from_ref(&dir), &Limits::default());

        write(&dir, "a.txt", "v2");
        run_git(&dir, &["commit", "-q", "-am", "edit in the run"]);
        let after = capture(std::slice::from_ref(&dir), &Limits::default());

        assert!(after.files.is_empty());
        assert_eq!(
            changed(&before, &after, &dir),
            vec![("a.txt".to_owned(), ChangeKind::Modified)]
        );
    }

    #[test]
    fn added_folder_is_scanned_with_its_own_mode() {
        let (_guard, main) = folder();
        let (_other_guard, extra) = folder();
        write(&main, "a.txt", "1");
        write(&extra, "b.txt", "1");
        let limits = Limits::default();
        let both = [main.clone(), extra.clone()];
        let before = capture(&both, &limits);

        write(&extra, "b.txt", "12");
        let after = capture(&both, &limits);

        let names: Vec<String> = diff(&before, &after, &[])
            .files
            .into_iter()
            .map(|change| change.path)
            .collect();
        assert_eq!(
            names,
            vec![extra.join("b.txt").to_string_lossy().into_owned()]
        );
    }

    #[test]
    fn file_count_limit_stops_the_scan_and_marks_it_partial() {
        let (_guard, dir) = folder();
        for index in 0..10 {
            write(&dir, &format!("f{index}.txt"), "x");
        }
        let limits = Limits {
            max_files: 4,
            ..Limits::default()
        };

        let snapshot = capture(&[dir], &limits);

        assert_eq!(snapshot.files.len(), 4);
        assert!(snapshot.is_partial);
    }

    #[test]
    fn expired_time_limit_marks_the_scan_partial() {
        let (_guard, dir) = folder();
        write(&dir, "a.txt", "x");
        let limits = Limits {
            max_time: Duration::ZERO,
            ..Limits::default()
        };

        let snapshot = capture(&[dir], &limits);

        assert!(snapshot.is_partial);
    }

    #[test]
    fn large_files_are_compared_without_a_hash() {
        let (_guard, dir) = folder();
        run_git(&dir, &["init", "-q"]);
        write(&dir, "big.bin", "0123456789");
        let limits = Limits {
            max_hash_bytes: 4,
            ..Limits::default()
        };

        let snapshot = capture(std::slice::from_ref(&dir), &limits);

        let state = snapshot
            .files
            .get(&dir.join("big.bin").to_string_lossy().into_owned())
            .unwrap();
        assert_eq!(state.digest, None);
        assert_eq!(state.size, 10);
    }
}
