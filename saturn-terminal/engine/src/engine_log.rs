//! engine 로그 파일. 하루(로컬 날짜) 한 파일에 쌓고, 30일 지난 파일은 시작 때와 날짜가 바뀔 때 지운다.
//! 설계: docs/design/engine-lifecycle.md

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use chrono::{Duration, Local, NaiveDate};
use tracing_subscriber::fmt::MakeWriter;

/// 로그 폴더. 홈 폴더 아래.
const LOG_DIR: &str = saturn_protocol::home::LOG_DIR;

const FILE_PREFIX: &str = "engine-";
const FILE_SUFFIX: &str = ".log";
const DATE_FORMAT: &str = "%Y-%m-%d";

/// 이 일수보다 오래된 날짜의 파일을 지운다. 근거: Claude Code `cleanupPeriodDays` 기본값.
const KEEP_DAYS: i64 = 30;

/// 날짜별 파일로 바꾸기 전에 쓰던 이름. 시작 때 지운다.
const LEGACY_FILE: &str = "engine.log";

/// `engine-YYYY-MM-DD.log`를 날짜가 바뀔 때마다 새로 여는 로그 쓰기 대상.
#[derive(Debug)]
pub struct EngineLog {
    dir: PathBuf,
    current: Mutex<OpenLog>,
}

#[derive(Debug)]
struct OpenLog {
    date: NaiveDate,
    file: File,
}

impl EngineLog {
    /// `home` 아래 로그 폴더를 만들고, 옛 로그와 30일 지난 파일을 지운 뒤 오늘 파일을 연다.
    ///
    /// # Errors
    /// 폴더나 파일을 만들거나 지울 수 없으면 오류.
    pub fn start(home: &Path) -> io::Result<Self> {
        Self::start_on(home, today())
    }

    fn start_on(home: &Path, today: NaiveDate) -> io::Result<Self> {
        let dir = home.join(LOG_DIR);
        std::fs::create_dir_all(&dir)?;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        remove_legacy_files(home, &dir)?;
        remove_expired_files(&dir, today)?;
        let file = open_file(&dir, today)?;
        Ok(Self {
            dir,
            current: Mutex::new(OpenLog { date: today, file }),
        })
    }

    fn write_on(&self, today: NaiveDate, buf: &[u8]) -> io::Result<usize> {
        let mut current = self.current.lock().unwrap_or_else(PoisonError::into_inner);
        if current.date != today {
            // 정리 실패가 로그 쓰기를 막지 않게 한다. 지우지 못한 파일은 다음 날짜 변경 때 다시 지운다.
            let _ = remove_expired_files(&self.dir, today);
            current.file = open_file(&self.dir, today)?;
            current.date = today;
        }
        current.file.write(buf)
    }

    fn flush_current(&self) -> io::Result<()> {
        let mut current = self.current.lock().unwrap_or_else(PoisonError::into_inner);
        current.file.flush()
    }
}

impl Write for &EngineLog {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.write_on(today(), buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flush_current()
    }
}

impl<'a> MakeWriter<'a> for EngineLog {
    type Writer = &'a EngineLog;

    fn make_writer(&'a self) -> Self::Writer {
        self
    }
}

fn today() -> NaiveDate {
    Local::now().date_naive()
}

fn file_name(date: NaiveDate) -> String {
    format!("{FILE_PREFIX}{}{FILE_SUFFIX}", date.format(DATE_FORMAT))
}

/// `engine-YYYY-MM-DD.log` 형식이면 그 날짜. 그 밖의 이름은 `None`.
fn date_of(name: &str) -> Option<NaiveDate> {
    let text = name.strip_prefix(FILE_PREFIX)?.strip_suffix(FILE_SUFFIX)?;
    let date = NaiveDate::parse_from_str(text, DATE_FORMAT).ok()?;
    (file_name(date) == name).then_some(date)
}

fn open_file(dir: &Path, date: NaiveDate) -> io::Result<File> {
    let path = dir.join(file_name(date));
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    Ok(file)
}

// cost: time O(n), heap O(1), stack O(1), io n
// vars: n = 로그 폴더의 파일 수
// basis: estimate
/// `today`보다 `KEEP_DAYS`일 넘게 앞선 날짜의 `engine-YYYY-MM-DD.log`만 지운다. 파일 날짜는 이름으로 읽는다.
fn remove_expired_files(dir: &Path, today: NaiveDate) -> io::Result<()> {
    let oldest_kept = today - Duration::days(KEEP_DAYS);
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let is_expired = entry
            .file_name()
            .to_str()
            .and_then(date_of)
            .is_some_and(|date| date < oldest_kept);
        if is_expired {
            std::fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

/// 날짜별 파일 이전의 `~/.saturn/engine.log`, `logs/engine.log`, `logs/engine.log.N`을 지운다.
fn remove_legacy_files(home: &Path, dir: &Path) -> io::Result<()> {
    remove_if_exists(&home.join(LEGACY_FILE))?;
    remove_if_exists(&dir.join(LEGACY_FILE))?;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let is_numbered = entry
            .file_name()
            .to_str()
            .and_then(|name| name.strip_prefix("engine.log."))
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
        if is_numbered {
            remove_if_exists(&entry.path())?;
        }
    }
    Ok(())
}

fn remove_if_exists(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn engine_log_writes_to_a_file_named_after_the_day() {
        let home = tempfile::tempdir().unwrap();
        let log = EngineLog::start_on(home.path(), date(2026, 10, 2)).unwrap();

        log.write_on(date(2026, 10, 2), b"first\n").unwrap();

        let logs = home.path().join("logs");
        assert_eq!(
            std::fs::read_to_string(logs.join("engine-2026-10-02.log")).unwrap(),
            "first\n"
        );
        assert_eq!(
            std::fs::metadata(logs.join("engine-2026-10-02.log"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(&logs).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }

    #[test]
    fn engine_log_opens_a_new_file_when_the_day_changes() {
        let home = tempfile::tempdir().unwrap();
        let log = EngineLog::start_on(home.path(), date(2026, 10, 2)).unwrap();

        log.write_on(date(2026, 10, 2), b"before\n").unwrap();
        log.write_on(date(2026, 10, 3), b"after\n").unwrap();

        let logs = home.path().join("logs");
        assert_eq!(
            std::fs::read_to_string(logs.join("engine-2026-10-02.log")).unwrap(),
            "before\n"
        );
        assert_eq!(
            std::fs::read_to_string(logs.join("engine-2026-10-03.log")).unwrap(),
            "after\n"
        );
    }

    #[test]
    fn engine_log_start_removes_only_files_older_than_30_days() {
        let home = tempfile::tempdir().unwrap();
        let logs = home.path().join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        for name in [
            "engine-2026-09-01.log",
            "engine-2026-09-02.log",
            "engine-2026-10-01.log",
            "engine-latest.log",
            "engine-2026-09-01.txt",
            "other-2026-01-01.log",
        ] {
            std::fs::write(logs.join(name), "x").unwrap();
        }

        EngineLog::start_on(home.path(), date(2026, 10, 2)).unwrap();

        assert_eq!(
            names(&logs),
            [
                "engine-2026-09-01.txt",
                "engine-2026-09-02.log",
                "engine-2026-10-01.log",
                "engine-2026-10-02.log",
                "engine-latest.log",
                "other-2026-01-01.log",
            ]
        );
    }

    #[test]
    fn engine_log_removes_files_older_than_30_days_when_the_day_changes() {
        let home = tempfile::tempdir().unwrap();
        let logs = home.path().join("logs");
        let log = EngineLog::start_on(home.path(), date(2026, 10, 2)).unwrap();
        std::fs::write(logs.join("engine-2026-09-02.log"), "x").unwrap();

        log.write_on(date(2026, 10, 3), b"next day\n").unwrap();

        assert_eq!(
            names(&logs),
            ["engine-2026-10-02.log", "engine-2026-10-03.log"]
        );
    }

    #[test]
    fn engine_log_start_removes_the_old_engine_log_files() {
        let home = tempfile::tempdir().unwrap();
        let logs = home.path().join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(home.path().join("engine.log"), "old").unwrap();
        for name in [
            "engine.log",
            "engine.log.1",
            "engine.log.5",
            "engine.log.bak",
        ] {
            std::fs::write(logs.join(name), "old").unwrap();
        }

        EngineLog::start_on(home.path(), date(2026, 10, 2)).unwrap();

        assert!(!home.path().join("engine.log").exists());
        assert_eq!(names(&logs), ["engine-2026-10-02.log", "engine.log.bak"]);
    }
}
