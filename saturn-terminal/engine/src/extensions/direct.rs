//! provider에 직접 설치된 항목 추적: 어댑터가 읽은 항목을 기록 저장소에 한 번만 물은 것으로 남기고, 사용자가 옮기겠다고
//! 하면 확장 저장소로 복사해 [설치](../lifecycle/extensions.rs)와 같은 판정을 한다. provider 폴더는 읽기만 한다.
//! 설계: docs/design/extensions.md#provider에-직접-설치한-것

use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use saturn_protocol::ids::{ChatId, Provider};
use saturn_protocol::rpc::{
    ChatNotice, DirectInstallInfo, DirectInstallItem, DirectKind, DirectState,
};

use super::ExtensionError;
use super::source::{self, copy_folder};
use crate::providers::{DirectInstall, DirectOrigin, part_kind_of};
use crate::rpc::ClientId;
use crate::{Engine, EngineError, masked_chain};

/// 기록 저장소 `direct_installs.kind`에 쓰는 글.
fn kind_text(kind: DirectKind) -> &'static str {
    match kind {
        DirectKind::Skill => "skill",
        DirectKind::Command => "command",
        DirectKind::McpServer => "mcp_server",
        DirectKind::Plugin => "plugin",
    }
}

fn state_of(text: &str) -> Option<DirectState> {
    match text {
        "asked" => Some(DirectState::Asked),
        "moved" => Some(DirectState::Moved),
        _ => None,
    }
}

impl Engine {
    /// 다른 어댑터 중 하나라도 이 종류를 주입할 수 있으면 옮길 수 있다. 플러그인은 부분이 아니라 옮기지 않는다.
    fn is_movable(&self, provider: Provider, item: &DirectInstall) -> bool {
        let Some(part) = part_kind_of(item.kind) else {
            return false;
        };
        self.registry.ids().into_iter().any(|other| {
            other != provider
                && self.registry.get(other).is_some_and(|adapter| {
                    adapter.injectability(part) == saturn_protocol::rpc::Injectability::Injectable
                })
        })
    }

    /// 한 provider의 직접 설치 항목. 어댑터를 모르면 비어 있다.
    fn read_direct(&self, provider: Provider, env: &[(OsString, OsString)]) -> Vec<DirectInstall> {
        self.registry
            .get(provider)
            .map(|adapter| adapter.direct_installs(env))
            .unwrap_or_default()
    }

    /// 연결을 시작할 때 부른다. 새로 찾은 항목을 한 번만 물은 것으로 기록하고 그 채팅의 대화 기록에 한 줄로 알린다.
    /// 읽거나 기록하지 못해도 연결 시작은 막지 않는다.
    pub(crate) async fn tell_direct_installs(
        &self,
        chat: ChatId,
        provider: Provider,
        env: &[(OsString, OsString)],
    ) {
        let found = self.read_direct(provider, env);
        if found.is_empty() {
            return;
        }
        let mut items = Vec::new();
        for item in &found {
            let recorded = self
                .store
                .record_direct_asked(provider.as_str(), kind_text(item.kind), &item.name)
                .await;
            match recorded {
                Ok(true) => items.push(DirectInstallItem {
                    kind: item.kind,
                    name: item.name.clone(),
                    movable: self.is_movable(provider, item),
                }),
                Ok(false) => {}
                Err(error) => {
                    self.warn_failure("failed to record a direct install", Err::<(), _>(error));
                    return;
                }
            }
        }
        if !items.is_empty() {
            self.notify_chat(chat, ChatNotice::DirectInstallsFound { provider, items })
                .await;
        }
    }

    /// 모든 어댑터의 직접 설치 항목과 기록한 상태. 어댑터 등록 순서대로다.
    ///
    /// # Errors
    /// 기록을 읽지 못하면 `Store`.
    pub(crate) async fn direct_install_infos(
        &self,
        env: &[(OsString, OsString)],
    ) -> Result<Vec<DirectInstallInfo>, EngineError> {
        let rows = self.store.direct_rows().await?;
        let mut infos = Vec::new();
        for provider in self.registry.ids() {
            for item in self.read_direct(provider, env) {
                let state = rows
                    .iter()
                    .find(|row| {
                        row.provider == provider.as_str()
                            && row.kind == kind_text(item.kind)
                            && row.name == item.name
                    })
                    .and_then(|row| state_of(&row.state));
                infos.push(DirectInstallInfo {
                    provider,
                    kind: item.kind,
                    movable: self.is_movable(provider, &item),
                    name: item.name,
                    state,
                });
            }
        }
        Ok(infos)
    }

    /// `MoveDirectExtension` 요청. 결과는 그 채팅의 대화 기록에 한 줄로 남기고, 실패해도 provider 폴더는 그대로다.
    ///
    /// # Errors
    /// 이 클라이언트가 붙지 않은 채팅이면 `ChatNotAttached`.
    pub(crate) async fn move_direct_extension(
        &mut self,
        client: ClientId,
        chat: ChatId,
        provider: Provider,
        name: &str,
    ) -> Result<(), EngineError> {
        self.require_attached(client, chat)?;
        let env = self
            .chat_env(chat)
            .map(crate::chat_env::ChatEnv::provider_env)
            .unwrap_or_default();
        let notice = match self.move_direct(provider, name, &env).await {
            Ok(extension) => {
                self.refresh_extension_connections().await;
                ChatNotice::ExtensionInstalled { extension }
            }
            Err(error) => ChatNotice::ExtensionFailed {
                name: Some(name.to_owned()),
                reason: masked_chain(&self.masker, &error),
            },
        };
        self.notify_chat(chat, notice).await;
        Ok(())
    }

    async fn move_direct(
        &mut self,
        provider: Provider,
        name: &str,
        env: &[(OsString, OsString)],
    ) -> Result<saturn_protocol::rpc::ExtensionInfo, ExtensionError> {
        let found = self.read_direct(provider, env);
        let named = found.iter().filter(|item| item.name == name);
        let item = named
            .clone()
            .find(|item| self.is_movable(provider, item))
            .or_else(|| named.clone().next())
            .ok_or(ExtensionError::InvalidSource {
                reason: "no such item is installed in the provider",
            })?
            .clone();
        if !self.is_movable(provider, &item) {
            return Err(ExtensionError::InvalidSource {
                reason: "this item cannot be moved to the extension store",
            });
        }
        let kind = item.kind;
        source::validate_name(name)?;
        let root = self.extension_store_dir();
        let dest = root.join(name);
        let taken = dest.exists()
            || self.flow.installing.contains(name)
            || self
                .store
                .extension_rows()
                .await?
                .iter()
                .any(|row| row.name == name);
        if taken {
            return Err(ExtensionError::AlreadyInstalled {
                name: name.to_owned(),
            });
        }
        std::fs::create_dir_all(&root).map_err(|source| ExtensionError::Read {
            path: root.display().to_string(),
            source,
        })?;
        let origin = format!("{} (installed in the provider)", provider.as_str());
        let placed = place_direct(&item.origin, name, &dest, &root);
        let info = self.finish_install(name, &origin, &dest, placed).await?;
        self.store
            .record_direct_moved(provider.as_str(), kind_text(kind), name)
            .await?;
        Ok(info)
    }
}

/// 원본을 확장 저장소 폴더 `dest`에 확장 한 개 모양으로 놓는다. 스킬은 루트의 `SKILL.md`, 명령은 `commands/`, MCP
/// 서버는 `.mcp.json`이다. 서버 정의에는 비밀이 들 수 있어 소유자만 읽게 한다.
fn place_direct(
    origin: &DirectOrigin,
    name: &str,
    dest: &Path,
    store: &Path,
) -> Result<(), ExtensionError> {
    let read = |path: &Path| {
        let path = path.display().to_string();
        move |source| ExtensionError::Read { path, source }
    };
    match origin {
        DirectOrigin::Folder(from) => copy_folder(from, dest, store),
        DirectOrigin::File(from) => {
            let dir = dest.join("commands");
            std::fs::create_dir_all(&dir).map_err(read(&dir))?;
            let file = dir.join(format!("{name}.md"));
            std::fs::copy(from, &file).map_err(read(&file))?;
            Ok(())
        }
        DirectOrigin::Server(definition) => {
            std::fs::create_dir_all(dest).map_err(read(dest))?;
            let file = dest.join(".mcp.json");
            let body = serde_json::json!({ "mcpServers": { name: definition } });
            std::fs::write(&file, body.to_string()).map_err(read(&file))?;
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600))
                .map_err(read(&file))?;
            Ok(())
        }
        DirectOrigin::TrackedOnly => Err(ExtensionError::InvalidSource {
            reason: "this item cannot be moved to the extension store",
        }),
    }
}
