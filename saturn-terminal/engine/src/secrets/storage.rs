//! 키 저장: macOS 키체인 OS API, 키체인이 없으면 0600 파일, 강화 방식의 잠금.
//! `security` 명령으로 저장하면 그 명령이 신뢰 앱이 되어 누구나 확인 창 없이 읽으므로 쓰지 않는다.
//! TODO(#102): 강화 방식 항목의 신뢰 앱 목록을 비우는 방법. 정해지기 전에는 잠금 시계만 더한다

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::{KeySource, ROUTER_KEY_ENV, RouterKey, SecretsError};

/// 마지막 사용 뒤 이만큼 쓰지 않으면 잠근다.
pub(crate) const HARDENED_IDLE_LOCK: Duration = Duration::from_secs(10 * 60);

/// 풀린 뒤 사용과 관계없이 이만큼 지나면 잠근다.
pub(crate) const HARDENED_MAX_UNLOCK: Duration = Duration::from_secs(12 * 60 * 60);

const KEYCHAIN_SERVICE: &str = "saturn";

const KEYCHAIN_ACCOUNT: &str = "saturn-key";

/// 훅 차단 목록도 이 이름을 쓴다. 초안 값.
pub(crate) const KEY_FILE: &str = "router.key";

/// 소유자만 읽고 쓴다.
const KEY_FILE_MODE: u32 = 0o600;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum StorageMode {
    #[default]
    Standard,
    /// 신뢰 앱 없는 항목으로 저장하고 session 시작 때 키체인 암호를 한 번 받는다.
    Hardened,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Backend {
    Keychain,
    /// 키체인을 쓸 수 없을 때. 경로는 초안이다.
    File(PathBuf),
}

/// engine에 하나. 키를 메모리에 들고 있는 곳은 여기뿐이다.
#[derive(Debug)]
pub(crate) struct SecretStore {
    backend: Backend,
    mode: StorageMode,
    /// 관리자 명령과 환경 변수 키도 여기에만 둔다.
    current: Option<(RouterKey, KeySource)>,
    /// 강화 방식에서 풀린 시각과 마지막 사용 시각.
    unlocked: Option<(Instant, Instant)>,
}

impl SecretStore {
    /// 키를 읽지는 않는다.
    pub(crate) fn open(home: &Path, mode: StorageMode) -> Self {
        let backend = if cfg!(target_os = "macos") {
            Backend::Keychain
        } else {
            Backend::File(home.join(KEY_FILE))
        };
        Self::with_backend(backend, mode)
    }

    /// 대체 파일 백엔드로 연다. 테스트가 키체인 대신 쓴다.
    #[cfg(test)]
    pub(crate) fn with_key_file(path: PathBuf, mode: StorageMode) -> Self {
        Self::with_backend(Backend::File(path), mode)
    }

    fn with_backend(backend: Backend, mode: StorageMode) -> Self {
        Self {
            backend,
            mode,
            current: None,
            unlocked: None,
        }
    }

    /// 환경 변수가 있으면 그것을 먼저 쓴다.
    ///
    /// # Errors
    /// 없으면 `NotFound`, 권한이 틀리면 `FilePermission`, 키체인 실패면 `Keychain`, 잠겼으면 `Locked`.
    pub(crate) async fn load(&mut self) -> Result<&RouterKey, SecretsError> {
        self.load_with_env(std::env::var(ROUTER_KEY_ENV).ok()).await
    }

    /// 테스트가 프로세스 환경을 바꾸지 않게 환경 변수 값을 밖에서 받는다.
    async fn load_with_env(
        &mut self,
        env_value: Option<String>,
    ) -> Result<&RouterKey, SecretsError> {
        if let Some(value) = env_value {
            let key = RouterKey::new(value)?;
            return Ok(&self.current.insert((key, KeySource::Env)).0);
        }
        if self.mode == StorageMode::Hardened && self.unlocked.is_none() {
            return Err(SecretsError::Locked);
        }
        let key = self.read_backend()?;
        Ok(&self.current.insert((key, KeySource::Stored)).0)
    }

    /// `Stored` 출처만 백엔드에 쓰고, `Env`와 `Command`는 메모리에만 둔다.
    ///
    /// # Errors
    /// 키체인 실패면 `Keychain`, 파일 실패면 `Io`.
    #[cfg(test)]
    pub(crate) async fn save(
        &mut self,
        key: RouterKey,
        source: KeySource,
    ) -> Result<(), SecretsError> {
        if source == KeySource::Stored {
            self.write_backend(&key)?;
        }
        self.current = Some((key, source));
        Ok(())
    }

    /// 확인 전 키라 백엔드에는 쓰지 않는다.
    pub(crate) fn hold(&mut self, key: RouterKey, source: KeySource) {
        self.current = Some((key, source));
    }

    /// `Stored` 출처만 쓴다.
    ///
    /// # Errors
    /// 키체인 실패면 `Keychain`, 파일 실패면 `Io`.
    pub(crate) fn persist_current(&mut self) -> Result<(), SecretsError> {
        match &self.current {
            Some((key, KeySource::Stored)) => self.write_backend(key),
            _ => Ok(()),
        }
    }

    /// 저장된 키는 그대로 둔다.
    pub(crate) fn drop_current(&mut self) {
        self.current = None;
    }

    /// # Errors
    /// 키체인 실패면 `Keychain`, 파일 삭제 실패면 `Io`.
    #[cfg(test)]
    pub(crate) async fn forget(&mut self) -> Result<(), SecretsError> {
        self.current = None;
        match &self.backend {
            Backend::Keychain => match keychain_entry()?.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(error) => Err(keychain_error(error)),
            },
            Backend::File(path) => match std::fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error.into()),
            },
        }
    }

    /// 강화 방식이면 잠금 조건을 먼저 확인하고 마지막 사용 시각을 갱신한다.
    ///
    /// # Errors
    /// 없으면 `NotFound`, 잠겼으면 `Locked`.
    pub(crate) fn key(&mut self, now: Instant) -> Result<&RouterKey, SecretsError> {
        let from_env = matches!(self.current, Some((_, KeySource::Env | KeySource::Command)));
        if self.mode == StorageMode::Hardened && !from_env {
            if self.lock_if_expired(now) {
                return Err(SecretsError::Locked);
            }
            let Some((since, _)) = self.unlocked else {
                return Err(SecretsError::Locked);
            };
            self.unlocked = Some((since, now));
        }
        self.current
            .as_ref()
            .map(|(key, _)| key)
            .ok_or(SecretsError::NotFound)
    }

    /// 암호는 OS 확인 창이 받고 Saturn은 보지 않는다.
    ///
    /// # Errors
    /// 사용자가 거부하면 `Locked`, 키체인 실패면 `Keychain`.
    pub(crate) async fn unlock(&mut self, now: Instant) -> Result<(), SecretsError> {
        if self.mode == StorageMode::Standard {
            return Ok(());
        }
        let key = match self.read_backend() {
            Ok(key) => key,
            Err(SecretsError::Keychain) => return Err(SecretsError::Locked),
            Err(error) => return Err(error),
        };
        self.current = Some((key, KeySource::Stored));
        self.unlocked = Some((now, now));
        Ok(())
    }

    /// 잠갔으면 참.
    pub(crate) fn lock_if_expired(&mut self, now: Instant) -> bool {
        if self.mode == StorageMode::Standard {
            return false;
        }
        let Some((since, last_use)) = self.unlocked else {
            return false;
        };
        let idle = now.saturating_duration_since(last_use) >= HARDENED_IDLE_LOCK;
        let too_long = now.saturating_duration_since(since) >= HARDENED_MAX_UNLOCK;
        if !(idle || too_long) {
            return false;
        }
        self.unlocked = None;
        if matches!(self.current, Some((_, KeySource::Stored))) {
            self.current = None;
        }
        true
    }

    pub(crate) fn mask_needles(&self) -> Vec<String> {
        self.current
            .iter()
            .map(|(key, _)| key.expose().to_owned())
            .collect()
    }

    fn read_backend(&self) -> Result<RouterKey, SecretsError> {
        match &self.backend {
            Backend::Keychain => match keychain_entry()?.get_password() {
                Ok(value) => RouterKey::new(value),
                Err(keyring::Error::NoEntry) => Err(SecretsError::NotFound),
                Err(error) => Err(keychain_error(error)),
            },
            Backend::File(path) => {
                let meta = match std::fs::metadata(path) {
                    Ok(meta) => meta,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        return Err(SecretsError::NotFound);
                    }
                    Err(error) => return Err(error.into()),
                };
                if meta.permissions().mode() & 0o777 != KEY_FILE_MODE {
                    return Err(SecretsError::FilePermission { path: path.clone() });
                }
                RouterKey::new(std::fs::read_to_string(path)?)
            }
        }
    }

    fn write_backend(&self, key: &RouterKey) -> Result<(), SecretsError> {
        match &self.backend {
            Backend::Keychain => keychain_entry()?
                .set_password(key.expose())
                .map_err(keychain_error),
            Backend::File(path) => {
                write_key_file(path, key.expose())?;
                Ok(())
            }
        }
    }
}

/// 키체인에 접근하지 않는다.
fn keychain_entry() -> Result<keyring::Entry, SecretsError> {
    keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT).map_err(keychain_error)
}

/// 원인 객체에 키가 담길 수 있어 그대로 보관하지 않는다.
fn keychain_error(_: keyring::Error) -> SecretsError {
    SecretsError::Keychain
}

/// 같은 폴더의 임시 파일을 0600으로 만들어 쓰고 이름을 바꿔 갈아 끼운다.
fn write_key_file(path: &Path, value: &str) -> std::io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("key file has no parent"))?;
    std::fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.as_file()
        .set_permissions(std::fs::Permissions::from_mode(KEY_FILE_MODE))?;
    file.write_all(value.as_bytes())?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    std::fs::File::open(parent)?.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_store(dir: &Path, mode: StorageMode) -> SecretStore {
        SecretStore::with_backend(Backend::File(dir.join(KEY_FILE)), mode)
    }

    fn key(value: &str) -> RouterKey {
        RouterKey::new(value.to_owned()).unwrap()
    }

    #[tokio::test]
    async fn file_backend_round_trips_with_0600() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = file_store(dir.path(), StorageMode::Standard);
        assert!(matches!(
            store.load_with_env(None).await,
            Err(SecretsError::NotFound)
        ));

        store
            .save(key("sk-file-1234"), KeySource::Stored)
            .await
            .unwrap();

        let mode = std::fs::metadata(dir.path().join(KEY_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, KEY_FILE_MODE);
        let mut reopened = file_store(dir.path(), StorageMode::Standard);
        assert_eq!(
            reopened.load_with_env(None).await.unwrap().expose(),
            "sk-file-1234"
        );
        assert_eq!(reopened.mask_needles(), vec!["sk-file-1234".to_owned()]);
        reopened.forget().await.unwrap();
        assert!(!dir.path().join(KEY_FILE).exists());
        assert!(matches!(
            reopened.key(Instant::now()),
            Err(SecretsError::NotFound)
        ));
    }

    #[tokio::test]
    async fn loose_file_permission_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(KEY_FILE);
        std::fs::write(&path, "sk-loose").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let mut store = file_store(dir.path(), StorageMode::Standard);

        let error = store.load_with_env(None).await.unwrap_err();

        assert!(matches!(error, SecretsError::FilePermission { .. }));
    }

    #[test]
    fn existing_partial_link_is_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let victim = dir.path().join("other.txt");
        std::fs::write(&victim, "keep this").unwrap();
        std::os::unix::fs::symlink(&victim, dir.path().join("router.key.partial")).unwrap();

        write_key_file(&dir.path().join(KEY_FILE), "sk-new").unwrap();

        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep this");
        assert_eq!(
            std::fs::read_to_string(dir.path().join(KEY_FILE)).unwrap(),
            "sk-new"
        );
    }

    #[tokio::test]
    async fn env_and_command_keys_stay_in_memory() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = file_store(dir.path(), StorageMode::Standard);

        let loaded = store
            .load_with_env(Some("sk-env".to_owned()))
            .await
            .unwrap();
        assert_eq!(loaded.expose(), "sk-env");
        store.save(key("sk-cmd"), KeySource::Command).await.unwrap();

        assert!(!dir.path().join(KEY_FILE).exists());
        assert_eq!(store.key(Instant::now()).unwrap().expose(), "sk-cmd");
    }

    #[tokio::test]
    async fn hardened_mode_locks_after_idle_and_max_unlock() {
        let dir = tempfile::tempdir().unwrap();
        file_store(dir.path(), StorageMode::Standard)
            .save(key("sk-hard"), KeySource::Stored)
            .await
            .unwrap();
        let mut store = file_store(dir.path(), StorageMode::Hardened);
        let start = Instant::now();
        assert!(matches!(
            store.load_with_env(None).await,
            Err(SecretsError::Locked)
        ));
        assert!(matches!(store.key(start), Err(SecretsError::Locked)));

        store.unlock(start).await.unwrap();
        let almost_idle = start + HARDENED_IDLE_LOCK - Duration::from_secs(1);
        assert_eq!(store.key(almost_idle).unwrap().expose(), "sk-hard");
        assert!(!store.lock_if_expired(almost_idle + Duration::from_secs(2)));
        assert!(store.lock_if_expired(almost_idle + HARDENED_IDLE_LOCK));
        assert!(store.mask_needles().is_empty());
        assert!(matches!(
            store.key(almost_idle + HARDENED_IDLE_LOCK),
            Err(SecretsError::Locked)
        ));

        store.unlock(start).await.unwrap();
        let mut now = start;
        while now < start + HARDENED_MAX_UNLOCK - Duration::from_secs(300) {
            now += Duration::from_secs(300);
            store.key(now).unwrap();
        }
        assert!(matches!(
            store.key(start + HARDENED_MAX_UNLOCK),
            Err(SecretsError::Locked)
        ));
    }

    #[tokio::test]
    async fn standard_mode_never_locks() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = file_store(dir.path(), StorageMode::Standard);
        store.save(key("sk-std"), KeySource::Stored).await.unwrap();
        store.unlock(Instant::now()).await.unwrap();

        let later = Instant::now() + HARDENED_MAX_UNLOCK * 2;

        assert!(!store.lock_if_expired(later));
        assert_eq!(store.key(later).unwrap().expose(), "sk-std");
    }

    #[test]
    fn bad_encoding_error_drops_stored_bytes() {
        let error = keychain_error(keyring::Error::BadEncoding(b"sk-secret".to_vec()));

        let shown = format!("{error:?} {error}");

        assert!(!shown.contains("sk-secret"));
        assert!(!shown.contains("115, 107"));
    }
}
