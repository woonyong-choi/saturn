//! 확장 저장소: 사용자가 설치한 확장의 원본을 `~/.saturn/extensions/`에 두고, 부분별 provider 사용 가능 판정을
//! 기록 저장소 `extensions` 표에 남긴다.
//! 설계: docs/design/extensions.md#확장-저장소

mod bundle;
mod direct;
mod inject;
pub(crate) mod source;

use std::path::PathBuf;

use saturn_protocol::ids::{ChatId, Provider};
use saturn_protocol::rpc::{
    ChatNotice, ExtensionInfo, ExtensionPart, ExtensionPartKind, Injectability, QueryResult,
    Request,
};
use serde::{Deserialize, Serialize};

use self::bundle::PartSpec;
use self::source::Source;
use crate::rpc::ClientId;
use crate::store::{ExtensionRow, StoreError};
use crate::{Engine, EngineError, masked_chain};

/// 확장 원본을 두는 폴더 이름. engine 홈 아래다.
const EXTENSIONS_DIR: &str = "extensions";

/// 설치하거나 지우지 못한 이유.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ExtensionError {
    #[error("invalid source: {reason}")]
    InvalidSource { reason: &'static str },
    #[error("invalid extension name: {name}")]
    InvalidName { name: String },
    #[error("extension {name} is already installed")]
    AlreadyInstalled { name: String },
    #[error("extension {name} is not installed")]
    NotInstalled { name: String },
    #[error("source has no skill, mcp server, command or hook")]
    NoParts,
    #[error("invalid definition in {file}: {reason}")]
    InvalidDefinition { file: String, reason: String },
    #[error("source is larger than {limit} bytes")]
    TooLarge { limit: u64 },
    #[error("failed to read {path}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("git clone failed: {detail}")]
    Clone { detail: String },
    #[error("git clone timed out")]
    CloneTimedOut,
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// 설치를 시작한 결과. 내려받는 중이면 끝날 때 `InstallDone`이 온다.
enum Started {
    Done(ExtensionInfo),
    Cloning,
}

/// 내려받기 작업이 요청 처리 루프에 돌려주는 결과.
#[derive(Debug)]
pub(crate) struct InstallDone {
    chat: ChatId,
    name: String,
    source: String,
    dest: PathBuf,
    placed: Result<(), ExtensionError>,
}

/// 기록 저장소 `extensions.parts`에 JSON으로 담는 부분 하나. 판정과 함께 주입에 쓸 위치를 가진다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct StoredPart {
    pub(super) kind: ExtensionPartKind,
    pub(super) name: String,
    /// 확장 폴더 안의 상대 경로.
    pub(super) path: String,
    verdicts: Vec<(Provider, Injectability)>,
}

impl StoredPart {
    fn info(&self) -> ExtensionPart {
        ExtensionPart {
            kind: self.kind,
            name: self.name.clone(),
            verdicts: self.verdicts.clone(),
        }
    }
}

pub(super) fn decode_parts(row: &ExtensionRow) -> Option<Vec<StoredPart>> {
    match serde_json::from_str(&row.parts) {
        Ok(parts) => Some(parts),
        Err(error) => {
            tracing::warn!(extension = %row.name, %error, "stored extension parts are unreadable");
            None
        }
    }
}

fn info_of(row: &ExtensionRow, parts: &[StoredPart]) -> ExtensionInfo {
    ExtensionInfo {
        name: row.name.clone(),
        source: row.source.clone(),
        installed_at_ms: u64::try_from(row.installed_at).unwrap_or_default(),
        parts: parts.iter().map(StoredPart::info).collect(),
    }
}

impl Engine {
    /// 확장 원본 폴더.
    pub(crate) fn extension_store_dir(&self) -> PathBuf {
        self.options.home.join(EXTENSIONS_DIR)
    }

    /// 등록한 어댑터마다 `kind` 부분을 주입할 수 있는지 묻는다. 어댑터 등록 순서대로다.
    fn judge(&self, kind: ExtensionPartKind) -> Vec<(Provider, Injectability)> {
        self.registry
            .ids()
            .into_iter()
            .filter_map(|provider| {
                let adapter = self.registry.get(provider)?;
                Some((provider, adapter.injectability(kind)))
            })
            .collect()
    }

    /// `InstallExtension` 요청. 결과는 성공이든 실패든 그 채팅의 대화 기록에 한 줄로 남긴다. 실패하면 기존 설치는
    /// 그대로다.
    ///
    /// # Errors
    /// 이 클라이언트가 붙지 않은 채팅이면 `ChatNotAttached`.
    pub(crate) async fn install_extension(
        &mut self,
        client: ClientId,
        chat: ChatId,
        source: &str,
    ) -> Result<(), EngineError> {
        self.require_attached(client, chat)?;
        let notice = match self.start_install(chat, source).await {
            Ok(Started::Done(extension)) => {
                self.refresh_extension_connections().await;
                ChatNotice::ExtensionInstalled { extension }
            }
            // 내려받기는 별도 작업이 하고, 끝나면 `on_install_done`이 결과를 알린다
            Ok(Started::Cloning) => return Ok(()),
            Err((name, error)) => ChatNotice::ExtensionFailed {
                name,
                reason: masked_chain(&self.masker, &error),
            },
        };
        self.notify_chat(chat, notice).await;
        Ok(())
    }

    /// 폴더는 바로 복사해 끝내고, git 주소는 내려받기를 별도 작업으로 돌린다. 느린 네트워크가 요청 처리 루프를 막지
    /// 않게 하기 위해서다. 실패하면 이름을 알아냈을 때 그 이름과 함께 돌려준다.
    async fn start_install(
        &mut self,
        chat: ChatId,
        source_text: &str,
    ) -> Result<Started, (Option<String>, ExtensionError)> {
        let source = Source::parse(source_text).map_err(|error| (None, error))?;
        let name = source.default_name().map_err(|error| (None, error))?;
        let fail = |error| (Some(name.clone()), error);
        source::validate_name(&name).map_err(fail)?;
        let root = self.extension_store_dir();
        let dest = root.join(&name);
        let known = self
            .store
            .extension_rows()
            .await
            .map_err(|e| fail(e.into()))?;
        if dest.exists()
            || self.flow.installing.contains(&name)
            || known.iter().any(|row| row.name == name)
        {
            return Err(fail(ExtensionError::AlreadyInstalled {
                name: name.clone(),
            }));
        }
        std::fs::create_dir_all(&root).map_err(|source| {
            fail(ExtensionError::Read {
                path: root.display().to_string(),
                source,
            })
        })?;
        match source {
            Source::Folder(from) => {
                let placed = source::copy_folder(&from, &dest, &root);
                self.finish_install(&name, source_text.trim(), &dest, placed)
                    .await
                    .map(Started::Done)
                    .map_err(fail)
            }
            Source::Git(url) => {
                self.flow.installing.insert(name.clone());
                let (tx, program) = (self.flow.install_tx.clone(), self.flow.git_program.clone());
                let (source, dest_path) = (source_text.trim().to_owned(), dest);
                tokio::spawn(async move {
                    let placed = source::clone_repository(&program, &url, &dest_path).await;
                    let _ = tx.send(InstallDone {
                        chat,
                        name,
                        source,
                        dest: dest_path,
                        placed,
                    }); // engine이 끝났으면 받을 곳이 없다
                });
                Ok(Started::Cloning)
            }
        }
    }

    /// 내려받기가 끝났다. 설치를 마무리하고 결과를 그 채팅의 대화 기록에 남긴다.
    pub(crate) async fn on_install_done(&mut self, done: InstallDone) {
        let InstallDone {
            chat,
            name,
            source,
            dest,
            placed,
        } = done;
        self.flow.installing.remove(&name);
        let notice = match self.finish_install(&name, &source, &dest, placed).await {
            Ok(extension) => {
                self.refresh_extension_connections().await;
                ChatNotice::ExtensionInstalled { extension }
            }
            Err(error) => ChatNotice::ExtensionFailed {
                reason: masked_chain(&self.masker, &error),
                name: Some(name),
            },
        };
        self.notify_chat(chat, notice).await;
    }

    /// 놓은 원본을 나누고 판정해 기록한다. 실패하면 이 요청이 만든 폴더만 지운다. 이름이 이미 있으면 시작할 때 거절해
    /// 여기까지 오지 않는다.
    async fn finish_install(
        &mut self,
        name: &str,
        source: &str,
        dest: &std::path::Path,
        placed: Result<(), ExtensionError>,
    ) -> Result<ExtensionInfo, ExtensionError> {
        let result = match placed {
            Ok(()) => self.record_install(name, source, dest).await,
            Err(error) => Err(error),
        };
        if result.is_err() && dest.exists() {
            let _ = std::fs::remove_dir_all(dest); // 지우지 못해도 설치 실패 알림이 먼저다
        }
        result
    }

    async fn record_install(
        &mut self,
        name: &str,
        source: &str,
        dest: &std::path::Path,
    ) -> Result<ExtensionInfo, ExtensionError> {
        let specs = bundle::split(dest, name)?;
        let parts: Vec<StoredPart> = specs
            .into_iter()
            .map(|PartSpec { kind, name, path }| StoredPart {
                verdicts: self.judge(kind),
                kind,
                name,
                path,
            })
            .collect();
        let encoded = serde_json::to_string(&parts).unwrap_or_else(|_| "[]".to_owned());
        if !self.store.insert_extension(name, source, &encoded).await? {
            return Err(ExtensionError::AlreadyInstalled {
                name: name.to_owned(),
            });
        }
        let rows = self.store.extension_rows().await?;
        let row = rows.iter().find(|row| row.name == name).ok_or_else(|| {
            ExtensionError::NotInstalled {
                name: name.to_owned(),
            }
        })?;
        Ok(info_of(row, &parts))
    }

    /// `RemoveExtension` 요청. 열린 session은 그대로이고 다음 session부터 빠진다.
    ///
    /// # Errors
    /// 이 클라이언트가 붙지 않은 채팅이면 `ChatNotAttached`.
    pub(crate) async fn remove_extension(
        &mut self,
        client: ClientId,
        chat: ChatId,
        name: &str,
    ) -> Result<(), EngineError> {
        self.require_attached(client, chat)?;
        let notice = match self.remove(name).await {
            Ok(()) => {
                self.refresh_extension_connections().await;
                ChatNotice::ExtensionRemoved {
                    name: name.to_owned(),
                }
            }
            Err(error) => ChatNotice::ExtensionFailed {
                name: Some(name.to_owned()),
                reason: masked_chain(&self.masker, &error),
            },
        };
        self.notify_chat(chat, notice).await;
        Ok(())
    }

    /// 이름은 기록에 있는 것만 지운다. 기록의 이름은 설치할 때 검사한 것이라 저장소 밖 경로가 될 수 없다.
    async fn remove(&mut self, name: &str) -> Result<(), ExtensionError> {
        if !self.store.delete_extension(name).await? {
            return Err(ExtensionError::NotInstalled {
                name: name.to_owned(),
            });
        }
        let dir = self.extension_store_dir().join(name);
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => Ok(()),
            // 원본이 이미 사라졌어도 기록은 지웠으니 제거는 끝났다
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(ExtensionError::Read {
                path: dir.display().to_string(),
                source,
            }),
        }
    }

    /// `ListExtensions` 요청의 답. 설치한 순서대로 요청한 접속에만 간다. 직접 설치 항목은 접속이 붙은 채팅의 환경으로 읽는다.
    ///
    /// # Errors
    /// 읽기 실패면 `Store`.
    pub(crate) async fn extension_list_result(
        &self,
        client: ClientId,
    ) -> Result<QueryResult, EngineError> {
        let env = self
            .attachments
            .get(&client)
            .and_then(|attachment| self.chat_env(attachment.chat))
            .map(crate::chat_env::ChatEnv::provider_env)
            .unwrap_or_default();
        let direct = self.direct_install_infos(&env).await?;
        let rows = self.store.extension_rows().await?;
        let extensions = rows
            .iter()
            .filter_map(|row| Some(info_of(row, &decode_parts(row)?)))
            .collect();
        Ok(QueryResult::ExtensionList { extensions, direct })
    }

    /// 확장 요청을 처리기로 나눈다. 확장 요청이 아니면 아무것도 하지 않는다.
    pub(crate) async fn route_extension(
        &mut self,
        client: ClientId,
        request: Request,
    ) -> Result<(), EngineError> {
        match request {
            Request::InstallExtension { chat, source } => {
                self.install_extension(client, chat, &source).await
            }
            Request::RemoveExtension { chat, name } => {
                self.remove_extension(client, chat, &name).await
            }
            Request::MoveDirectExtension {
                chat,
                provider,
                name,
            } => {
                self.move_direct_extension(client, chat, provider, &name)
                    .await
            }
            _ => Ok(()),
        }
    }

    /// 시작할 때 `Unknown`으로 남은 판정을 어댑터에 다시 묻는다. 어댑터를 고치면 판정이 바뀌기 때문이다. 실패는
    /// 시작을 막지 않고 로그만 남긴다.
    pub(crate) async fn rejudge_unknown_extensions(&mut self) {
        let rows = match self.store.extension_rows().await {
            Ok(rows) => rows,
            Err(error) => {
                self.warn_failure("failed to read installed extensions", Err::<(), _>(error));
                return;
            }
        };
        for row in rows {
            let Some(mut parts) = decode_parts(&row) else {
                continue;
            };
            if !self.rejudge(&mut parts) {
                continue;
            }
            if let Ok(encoded) = serde_json::to_string(&parts) {
                let saved = self.store.update_extension_parts(&row.name, &encoded).await;
                self.warn_failure("failed to save the extension judgment", saved);
            }
        }
    }

    /// `Unknown` 판정만 다시 묻는다. 바뀐 것이 있으면 `true`.
    fn rejudge(&self, parts: &mut [StoredPart]) -> bool {
        let mut changed = false;
        for part in parts {
            let kind = part.kind;
            for (provider, verdict) in &mut part.verdicts {
                if *verdict != Injectability::Unknown {
                    continue;
                }
                if let Some(adapter) = self.registry.get(*provider) {
                    let fresh = adapter.injectability(kind);
                    changed |= fresh != *verdict;
                    *verdict = fresh;
                }
            }
        }
        changed
    }
}
