//! 확장 주입 계획: 설치한 확장 중 provider가 주입 가능하다고 답한 부분을 모아 어댑터에 넘긴다.
//! 설계: docs/design/extensions.md#주입

use std::collections::HashMap;
use std::path::PathBuf;

use saturn_protocol::ids::{ChatId, Provider};
use saturn_protocol::rpc::{ChatNotice, ExtensionPartKind, Injectability};

use super::{StoredPart, decode_parts};
use crate::Engine;
use crate::providers::{InjectedPart, PermissionLaunch};
use crate::store::{StoreError, sha256_hex};

/// 연결 폴더 이름에 붙는 지문의 글자 수. 초안.
const FINGERPRINT_LEN: usize = 12;

/// 한 provider에 넘길 부분과 그 지문, 원본이 사라져 건너뛴 확장.
#[derive(Debug, Default)]
pub(crate) struct ExtensionPlan {
    pub(crate) parts: Vec<InjectedPart>,
    /// 부분이 없으면 빈 글자다.
    pub(crate) fingerprint: String,
    /// 원본 폴더가 확장 저장소에서 사라진 확장 이름.
    pub(crate) missing: Vec<String>,
}

impl Engine {
    // cost: time O(e·p), heap O(e·p), stack O(1), io e
    // vars: e = 설치한 확장 수, p = 확장당 부분 수
    // basis: estimate
    /// `provider`에 주입할 부분. 어댑터에 지금 다시 물어 `주입 가능`인 부분만 담고, 기록에 저장한 판정은 쓰지 않는다.
    /// 어댑터를 고치면 판정이 바뀌기 때문이다. 원본 폴더가 사라진 확장은 건너뛰고 `missing`에 적는다.
    ///
    /// # Errors
    /// 확장 표를 읽지 못하면 `StoreError`.
    pub(crate) async fn extension_plan(
        &self,
        provider: Provider,
    ) -> Result<ExtensionPlan, StoreError> {
        let mut plan = ExtensionPlan::default();
        let Some(adapter) = self.registry.get(provider) else {
            return Ok(plan);
        };
        let store = self.extension_store_dir();
        let mut lines = String::new();
        for row in self.store.extension_rows().await? {
            let Some(parts) = decode_parts(&row) else {
                continue;
            };
            let root = store.join(&row.name);
            let usable: Vec<&StoredPart> = parts
                .iter()
                .filter(|part| adapter.injectability(part.kind) == Injectability::Injectable)
                .collect();
            if usable.is_empty() {
                continue;
            }
            if !root.is_dir() {
                plan.missing.push(row.name.clone());
                continue;
            }
            for part in usable {
                lines.push_str(&format!(
                    "{}\t{}\t{:?}\t{}\t{}\n",
                    row.name, row.installed_at, part.kind, part.name, part.path
                ));
                plan.parts.push(InjectedPart {
                    extension: row.name.clone(),
                    kind: part.kind,
                    name: part.name.clone(),
                    source: part_source(&root, &part.path),
                });
            }
        }
        if !plan.parts.is_empty() {
            let mut digest = sha256_hex(lines.as_bytes());
            digest.truncate(FINGERPRINT_LEN);
            plan.fingerprint = digest;
        }
        Ok(plan)
    }

    /// 주입하지 못한 부분을 연결을 시작하는 채팅의 대화 기록에 한 줄씩 남긴다.
    pub(crate) async fn tell_injection_failures(
        &self,
        chat: ChatId,
        provider: Provider,
        plan: &ExtensionPlan,
        permission: &PermissionLaunch,
    ) {
        for extension in &plan.missing {
            self.notify_chat(
                chat,
                ChatNotice::ExtensionInjectFailed {
                    extension: extension.clone(),
                    part: None,
                    provider,
                    reason: "the original is missing from the extension store".to_owned(),
                },
            )
            .await;
        }
        for failure in &permission.injection_failures {
            self.notify_chat(
                chat,
                ChatNotice::ExtensionInjectFailed {
                    extension: failure.extension.clone(),
                    part: Some(failure.part.clone()),
                    provider,
                    reason: self.masker.mask(&failure.reason).as_str().to_owned(),
                },
            )
            .await;
        }
    }

    // cost: time O(e·p), heap O(e·p), stack O(1)
    // vars: e = 설치한 확장 수, p = 확장당 부분 수
    // basis: estimate
    /// provider가 `from`에서 `to`로 바뀔 때 `to`가 받지 못하는 부분을 대화 기록에 알린다. `from`에는 주입했고 `to`에는
    /// 주입하지 못하는 부분만이다. 처음부터 어느 쪽도 쓰지 못하던 부분은 설치할 때 알렸으므로 다시 알리지 않는다. 없으면
    /// 아무것도 보내지 않는다. 알림 실패가 전환을 막지 않게 읽기 실패는 로그만 남긴다.
    pub(crate) async fn tell_parts_left_behind(&self, chat: ChatId, from: Provider, to: Provider) {
        let (Some(old), Some(new)) = (self.registry.get(from), self.registry.get(to)) else {
            return;
        };
        let rows = match self.store.extension_rows().await {
            Ok(rows) => rows,
            Err(error) => {
                self.warn_failure("failed to read installed extensions", Err::<(), _>(error));
                return;
            }
        };
        let parts: Vec<(String, ExtensionPartKind, String)> = rows
            .iter()
            .filter_map(|row| Some((row, decode_parts(row)?)))
            .flat_map(|(row, parts)| {
                parts
                    .into_iter()
                    .filter(|part| {
                        old.injectability(part.kind) == Injectability::Injectable
                            && new.injectability(part.kind) != Injectability::Injectable
                    })
                    .map(|part| (row.name.clone(), part.kind, part.name))
                    .collect::<Vec<_>>()
            })
            .collect();
        if !parts.is_empty() {
            self.notify_chat(
                chat,
                ChatNotice::ExtensionPartsNotApplied {
                    provider: to,
                    parts,
                },
            )
            .await;
        }
    }

    /// 설치나 제거로 주입할 부분이 바뀐 연결을 표시하고, 작업이 없는 채팅은 바로 다시 시작한다. 작업 중인 채팅은 턴
    /// 끝에 다시 시작한다. 열린 session은 그대로 두고 다음 session부터 새 구성이 적용된다. 지문을 읽지 못한 연결은
    /// 그대로 둔다.
    pub(crate) async fn refresh_extension_connections(&mut self) {
        let keys: Vec<(ChatId, Provider)> = self.providers.keys().copied().collect();
        let mut current: HashMap<Provider, Option<String>> = HashMap::new();
        for provider in keys.iter().map(|(_, provider)| *provider) {
            if current.contains_key(&provider) {
                continue;
            }
            let plan = self.extension_plan(provider).await;
            let fingerprint = match plan {
                Ok(plan) => Some(plan.fingerprint),
                Err(error) => {
                    self.warn_failure("failed to read installed extensions", Err::<(), _>(error));
                    None
                }
            };
            current.insert(provider, fingerprint);
        }
        let mut chats = Vec::new();
        for (chat, provider) in keys {
            let Some(Some(now)) = current.get(&provider) else {
                continue;
            };
            let started = self
                .flow
                .extensions_of_connection
                .get(&(chat, provider))
                .map_or("", String::as_str);
            if started != now {
                self.flow.stale_connections.insert((chat, provider));
                chats.push(chat);
            }
        }
        chats.sort();
        chats.dedup();
        for chat in chats {
            self.restart_stale_connections(chat).await;
        }
    }
}

/// 부분 위치를 저장소 안의 절대 경로로 바꾼다. `.`은 확장 폴더 자체다.
fn part_source(root: &std::path::Path, relative: &str) -> PathBuf {
    if relative == "." {
        root.to_path_buf()
    } else {
        root.join(relative)
    }
}
