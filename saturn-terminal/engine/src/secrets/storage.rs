//! 키 저장: macOS 키체인 OS API, 키체인이 없으면 0600 파일, 강화 방식의 신뢰 앱 없는 항목과 잠금.
//! `security` 명령으로 저장하면 그 명령이 신뢰 앱이 되어 누구나 확인 창 없이 읽으므로 쓰지 않는다.
//! 강화 방식은 macOS 키체인의 빈 신뢰 앱 목록 항목으로만 지원하고, 지원되지 않거나 검증에 실패하면
//! 표준 저장으로 낮추지 않고 `HardenedUnsupported`로 끝낸다.

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::{KeySource, ROUTER_KEY_ENV, RouterKey, SecretsError};

/// 강화 항목 하나에 대한 OS 호출 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ItemError {
    NotFound,
    Duplicate,
    /// 사용자가 확인 창을 닫았다.
    Cancelled,
    /// 사용자가 거부했거나 인증에 실패했다.
    Denied,
    /// 확인 창이 필요한데 띄울 수 없는 상태다.
    InteractionRequired,
    /// 만든 항목의 접근 정책이 강화 기준과 다르다.
    PolicyMismatch,
    Failed,
}

/// 신뢰 앱 없는 키체인 항목 하나. macOS 어댑터가 구현하고, 다른 OS에는 없다.
pub(super) trait HardenedKeychain: std::fmt::Debug + Send {
    /// 읽기에 확인 창이 필요한 항목을 만든다.
    fn create(&self, value: &str) -> Result<(), ItemError>;
    /// 확인 창이 뜰 수 있다.
    fn read(&self) -> Result<String, ItemError>;
    fn delete(&self) -> Result<(), ItemError>;
    /// 만든 항목이 실제로 확인 없이 읽히지 않는지 확인한다. 아니면 `PolicyMismatch`.
    fn verify_policy(&self) -> Result<(), ItemError>;
}

#[cfg(target_os = "macos")]
const HARDENED_ACCOUNT: &str = "saturn-key-hardened";

/// 마지막 사용 뒤 이만큼 쓰지 않으면 잠근다.
pub(crate) const HARDENED_IDLE_LOCK: Duration = Duration::from_secs(10 * 60);

/// 풀린 뒤 사용과 관계없이 이만큼 지나면 잠근다.
pub(crate) const HARDENED_MAX_UNLOCK: Duration = Duration::from_secs(12 * 60 * 60);

/// 초안 값.
pub(crate) const LOCK_CHECK_INTERVAL: Duration = Duration::from_secs(60);

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
    /// 신뢰 앱 없는 항목으로 저장하고 session 시작 때 OS 확인을 한 번 받는다.
    /// 지원하지 않는 환경에서는 표준으로 낮추지 않고 오류로 끝난다.
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
    /// 강화 방식을 지원하는 환경에만 있다.
    hardened: Option<Box<dyn HardenedKeychain>>,
}

impl SecretStore {
    /// 키를 읽지는 않는다.
    pub(crate) fn open(home: &Path, mode: StorageMode) -> Self {
        let backend = if cfg!(target_os = "macos") {
            Backend::Keychain
        } else {
            Backend::File(home.join(KEY_FILE))
        };
        #[cfg(target_os = "macos")]
        let hardened: Option<Box<dyn HardenedKeychain>> = Some(Box::new(
            super::keychain::MacKeychainItem::new(KEYCHAIN_SERVICE, HARDENED_ACCOUNT),
        ));
        #[cfg(not(target_os = "macos"))]
        let hardened = None;
        Self::with_backend(backend, mode, hardened)
    }

    /// 대체 파일 백엔드로 연다. 테스트가 키체인 대신 쓴다.
    #[cfg(test)]
    pub(crate) fn with_key_file(path: PathBuf, mode: StorageMode) -> Self {
        Self::with_backend(Backend::File(path), mode, None)
    }

    fn with_backend(
        backend: Backend,
        mode: StorageMode,
        hardened: Option<Box<dyn HardenedKeychain>>,
    ) -> Self {
        Self {
            backend,
            mode,
            current: None,
            unlocked: None,
            hardened,
        }
    }

    /// 환경 변수가 있으면 그것을 먼저 쓴다.
    ///
    /// # Errors
    /// 없으면 `NotFound`, 권한이 틀리면 `FilePermission`, 키체인 실패면 `Keychain`, 잠겼으면 `Locked`,
    /// 강화 방식을 지원하지 않으면 `HardenedUnsupported`.
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
        if self.mode == StorageMode::Hardened {
            self.hardened_item()?;
            // 풀린 뒤에는 메모리의 키를 쓴다. 다시 읽으면 확인 창이 또 뜬다.
            return match (&self.current, self.unlocked) {
                (Some((key, KeySource::Stored)), Some(_)) => Ok(key),
                _ => Err(SecretsError::Locked),
            };
        }
        let key = self.read_standard()?;
        Ok(&self.current.insert((key, KeySource::Stored)).0)
    }

    /// `Stored` 출처만 백엔드에 쓰고, `Env`와 `Command`는 메모리에만 둔다.
    ///
    /// # Errors
    /// 키체인 실패면 `Keychain`, 파일 실패면 `Io`.
    pub(crate) async fn save(
        &mut self,
        key: RouterKey,
        source: KeySource,
    ) -> Result<(), SecretsError> {
        if source == KeySource::Stored {
            self.write_backend(&key)?;
            if self.mode == StorageMode::Hardened {
                let now = Instant::now();
                self.unlocked = Some((now, now));
            }
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
        let Some((key, KeySource::Stored)) = &self.current else {
            return Ok(());
        };
        self.write_backend(key)?;
        if self.mode == StorageMode::Hardened {
            let now = Instant::now();
            self.unlocked = Some((now, now));
        }
        Ok(())
    }

    /// 저장된 키는 그대로 둔다.
    pub(crate) fn drop_current(&mut self) {
        self.current = None;
    }

    /// 표준과 강화 항목을 모두 지운다.
    ///
    /// # Errors
    /// 키체인 실패면 `Keychain`, 파일 삭제 실패면 `Io`.
    pub(crate) async fn forget(&mut self) -> Result<(), SecretsError> {
        self.current = None;
        self.unlocked = None;
        let standard = self.delete_standard();
        if let Some(item) = &self.hardened {
            item.delete().map_err(|_| SecretsError::Keychain)?;
        }
        standard
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

    /// 확인은 OS 창이 받고 Saturn은 보지 않는다. 강화 항목이 없고 표준 항목이 있으면 옮긴다.
    ///
    /// # Errors
    /// 사용자가 거부하거나 취소하거나 창을 띄울 수 없으면 `Locked`, 키체인 실패면 `Keychain`,
    /// 지원하지 않거나 만든 항목이 기준에 못 미치면 `HardenedUnsupported`.
    pub(crate) async fn unlock(&mut self, now: Instant) -> Result<(), SecretsError> {
        if self.mode == StorageMode::Standard {
            return Ok(());
        }
        let read = self.hardened_item()?.read();
        let key = match read {
            Ok(value) => RouterKey::new(value)?,
            Err(ItemError::NotFound) => return self.migrate_standard(now),
            Err(ItemError::Cancelled | ItemError::Denied | ItemError::InteractionRequired) => {
                return Err(SecretsError::Locked);
            }
            Err(_) => return Err(SecretsError::Keychain),
        };
        self.current = Some((key, KeySource::Stored));
        self.unlocked = Some((now, now));
        Ok(())
    }

    /// 새 항목을 만들고 접근 정책을 확인한 뒤에만 기존 항목을 지운다.
    /// 그 앞에서 실패하면 기존 항목은 그대로 둔다.
    fn migrate_standard(&mut self, now: Instant) -> Result<(), SecretsError> {
        let key = self.read_standard()?;
        self.write_hardened(&key)?;
        self.current = Some((key, KeySource::Stored));
        self.unlocked = Some((now, now));
        self.delete_standard()
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

    fn hardened_item(&self) -> Result<&dyn HardenedKeychain, SecretsError> {
        self.hardened
            .as_deref()
            .ok_or(SecretsError::HardenedUnsupported)
    }

    fn read_standard(&self) -> Result<RouterKey, SecretsError> {
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

    fn delete_standard(&self) -> Result<(), SecretsError> {
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

    fn write_backend(&self, key: &RouterKey) -> Result<(), SecretsError> {
        if self.mode == StorageMode::Hardened {
            self.write_hardened(key)?;
            // 새 항목을 확인한 뒤에만 낮은 보호의 옛 사본을 지운다.
            return self.delete_standard();
        }
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

    /// 만든 항목이 기준에 못 미치면 지우고 `HardenedUnsupported`로 끝낸다. 표준 항목은 건드리지 않는다.
    fn write_hardened(&self, key: &RouterKey) -> Result<(), SecretsError> {
        let item = self.hardened_item()?;
        item.delete().map_err(|_| SecretsError::Keychain)?;
        item.create(key.expose())
            .map_err(|_| SecretsError::Keychain)?;
        if item.verify_policy().is_err() {
            let _ = item.delete();
            return Err(SecretsError::HardenedUnsupported);
        }
        Ok(())
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
        SecretStore::with_backend(Backend::File(dir.join(KEY_FILE)), mode, None)
    }

    /// 미리 정한 결과를 돌려주는 강화 항목. 확인 창 없이 허용·거부·취소·정책 불일치를 만든다.
    #[derive(Debug, Default)]
    struct FakeState {
        value: Option<String>,
        read_error: Option<ItemError>,
        create_error: Option<ItemError>,
        policy_error: Option<ItemError>,
        reads: usize,
    }

    #[derive(Debug, Clone, Default)]
    struct FakeItem(std::sync::Arc<std::sync::Mutex<FakeState>>);

    impl FakeItem {
        fn state(&self) -> std::sync::MutexGuard<'_, FakeState> {
            self.0.lock().unwrap()
        }
    }

    impl HardenedKeychain for FakeItem {
        fn create(&self, value: &str) -> Result<(), ItemError> {
            let mut state = self.state();
            if let Some(error) = state.create_error {
                return Err(error);
            }
            state.value = Some(value.to_owned());
            Ok(())
        }

        fn read(&self) -> Result<String, ItemError> {
            let mut state = self.state();
            state.reads += 1;
            if let Some(error) = state.read_error {
                return Err(error);
            }
            state.value.clone().ok_or(ItemError::NotFound)
        }

        fn delete(&self) -> Result<(), ItemError> {
            self.state().value = None;
            Ok(())
        }

        fn verify_policy(&self) -> Result<(), ItemError> {
            self.state().policy_error.map_or(Ok(()), Err)
        }
    }

    /// 표준 저장은 파일, 강화 저장은 가짜 항목으로 연다.
    fn hardened_store(dir: &Path, item: &FakeItem) -> SecretStore {
        SecretStore::with_backend(
            Backend::File(dir.join(KEY_FILE)),
            StorageMode::Hardened,
            Some(Box::new(item.clone())),
        )
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
        let item = FakeItem::default();
        item.state().value = Some("sk-hard".to_owned());
        let mut store = hardened_store(dir.path(), &item);
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
        assert_eq!(item.state().reads, 1);

        store.unlock(start).await.unwrap();
        assert_eq!(item.state().reads, 2, "잠근 뒤 접근은 항목을 다시 읽는다");
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
    async fn hardened_unlock_outcomes_never_fall_back_to_standard() {
        let rows = [
            (Some(ItemError::Denied), "Locked"),
            (Some(ItemError::Cancelled), "Locked"),
            (Some(ItemError::InteractionRequired), "Locked"),
            (Some(ItemError::Failed), "Keychain"),
            (None, "Ok"),
        ];
        for (read_error, expected) in rows {
            let dir = tempfile::tempdir().unwrap();
            let item = FakeItem::default();
            item.state().value = Some("sk-hard".to_owned());
            item.state().read_error = read_error;
            std::fs::write(dir.path().join(KEY_FILE), "sk-std").unwrap();
            std::fs::set_permissions(
                dir.path().join(KEY_FILE),
                std::fs::Permissions::from_mode(KEY_FILE_MODE),
            )
            .unwrap();
            let mut store = hardened_store(dir.path(), &item);

            let result = store.unlock(Instant::now()).await;

            let outcome = match &result {
                Ok(()) => "Ok",
                Err(SecretsError::Locked) => "Locked",
                Err(SecretsError::Keychain) => "Keychain",
                Err(_) => "other",
            };
            assert_eq!(outcome, expected, "{read_error:?}");
            let loaded = store.key(Instant::now()).map(|key| key.expose().to_owned());
            if result.is_ok() {
                assert_eq!(loaded.unwrap(), "sk-hard");
            } else {
                assert!(matches!(loaded, Err(SecretsError::Locked)));
                assert!(store.mask_needles().is_empty());
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let mut unsupported =
            SecretStore::with_key_file(dir.path().join(KEY_FILE), StorageMode::Hardened);
        assert!(matches!(
            unsupported.unlock(Instant::now()).await,
            Err(SecretsError::HardenedUnsupported)
        ));
        assert!(matches!(
            unsupported.load_with_env(None).await,
            Err(SecretsError::HardenedUnsupported)
        ));
        assert!(matches!(
            unsupported.save(key("sk-new"), KeySource::Stored).await,
            Err(SecretsError::HardenedUnsupported)
        ));
        assert!(!dir.path().join(KEY_FILE).exists());
    }

    #[tokio::test]
    async fn hardened_migration_keeps_standard_item_until_new_item_is_verified() {
        let rows = [
            (None, None, true),
            (Some(ItemError::Failed), None, false),
            (None, Some(ItemError::PolicyMismatch), false),
        ];
        for (create_error, policy_error, migrated) in rows {
            let dir = tempfile::tempdir().unwrap();
            file_store(dir.path(), StorageMode::Standard)
                .save(key("sk-std"), KeySource::Stored)
                .await
                .unwrap();
            let item = FakeItem::default();
            item.state().create_error = create_error;
            item.state().policy_error = policy_error;
            let mut store = hardened_store(dir.path(), &item);

            let result = store.unlock(Instant::now()).await;

            let standard_left = dir.path().join(KEY_FILE).exists();
            if migrated {
                result.unwrap();
                assert!(!standard_left);
                assert_eq!(item.state().value.as_deref(), Some("sk-std"));
                assert_eq!(store.key(Instant::now()).unwrap().expose(), "sk-std");
            } else {
                assert!(result.is_err());
                assert!(standard_left, "실패하면 기존 항목이 남는다");
                assert_eq!(item.state().value, None, "기준 미달 항목은 남기지 않는다");
                assert!(matches!(
                    store.key(Instant::now()),
                    Err(SecretsError::Locked)
                ));
                let reopened = file_store(dir.path(), StorageMode::Standard)
                    .read_standard()
                    .unwrap();
                assert_eq!(reopened.expose(), "sk-std");
            }
        }
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
