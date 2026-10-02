//! 고를 수 있는 모델 목록과 입력에 고정한 모델 읽기.
//! 설계: docs/design/providers-and-sessions.md#모델-고르기

use saturn_core::providers::ProviderClient;
use saturn_core::queue::QueuedInput;
use saturn_protocol::ids::{ChatId, Provider, SettingsRevision};
use saturn_protocol::rpc::{ModelChoice, ModelInfo, Notification};

use crate::providers::{FIRST_INPUT_ORDER, is_installed, parse_pinned};
use crate::rpc::ClientId;
use crate::settings::SettingsError;
use crate::{Engine, EngineError, masked_chain};

/// 입력에 고정한 모델. provider 접두사가 없는 글은 모델 이름으로만 읽어 provider를 정하지 않는다.
pub(crate) fn pinned_choice(record: &QueuedInput) -> Option<ModelChoice> {
    record.pinned_model.as_deref().and_then(parse_pinned)
}

/// provider에 넘길 모델 이름.
pub(crate) fn pinned_model_name(record: &QueuedInput) -> Option<String> {
    match pinned_choice(record) {
        Some(choice) => Some(choice.model),
        None => record.pinned_model.clone(),
    }
}

impl Engine {
    // cost: time O(m), heap O(m), stack O(1), io p
    // vars: m = 모델 수, p = provider 수
    // basis: estimate
    /// 설치된 provider마다 모델 목록을 받아 `Models`로 보낸다. 목록을 못 받은 provider는 건너뛰고 로그만 남긴다.
    /// 모든 provider가 실패하면 빈 목록을 보내고 마지막 오류를 돌려준다.
    ///
    /// # Errors
    /// 붙지 않은 채팅이면 `ChatNotAttached`, 설정이 없으면 `Settings`, 모든 provider의 목록이 실패하면 `Provider`.
    pub(crate) async fn send_models(
        &mut self,
        client: ClientId,
        chat: ChatId,
        only: Option<Provider>,
    ) -> Result<(), EngineError> {
        self.attached_workdir(client, chat)?;
        let revision = self
            .settings
            .current()
            .ok_or(SettingsError::NoPreviousRevision)?;
        let env = self
            .chat_env(chat)
            .map(crate::chat_env::ChatEnv::provider_env)
            .unwrap_or_default();
        let mut models: Vec<ModelInfo> = Vec::new();
        let mut failure = None;
        for provider in FIRST_INPUT_ORDER
            .into_iter()
            .filter(|provider| only.is_none_or(|only| only == *provider))
        {
            if !self.providers.contains_key(&(chat, provider)) && !is_installed(provider, &env) {
                continue;
            }
            match self.list_provider_models(provider, chat, revision).await {
                Ok(list) => models.extend(list),
                Err(error) => {
                    tracing::warn!(error = %masked_chain(&self.masker, &error), "failed to list models");
                    failure = Some(error);
                }
            }
        }
        // 창이 끝없이 기다리지 않도록 실패해도 빈 목록은 보낸다.
        self.send(
            client,
            Notification::Models {
                models: models.clone(),
            },
        )
        .await;
        match failure {
            Some(error) if models.is_empty() => Err(error),
            _ => Ok(()),
        }
    }

    async fn list_provider_models(
        &mut self,
        provider: Provider,
        chat: ChatId,
        revision: SettingsRevision,
    ) -> Result<Vec<ModelInfo>, EngineError> {
        self.ensure_connected(provider, chat, revision).await?;
        Ok(self.provider_mut(chat, provider)?.list_models().await?)
    }
}
