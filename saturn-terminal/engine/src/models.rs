//! 고를 수 있는 모델 목록과 입력에 고정한 모델 읽기.
//! 설계: docs/design/providers-and-sessions.md#모델-고르기

use saturn_core::providers::ProviderError;
use saturn_core::queue::QueuedInput;
use saturn_protocol::ids::{ChatId, Provider, SettingsRevision};
use saturn_protocol::rpc::{ModelChoice, ModelInfo, Notification};

use crate::providers::Registry;
use crate::rpc::ClientId;
use crate::settings::SettingsError;
use crate::{Engine, EngineError, masked_chain};

/// 입력에 고정한 모델. provider 접두사가 없거나 등록하지 않은 id인 글은 모델 이름으로만 읽어 provider를 정하지 않는다.
pub(crate) fn pinned_choice(registry: &Registry, record: &QueuedInput) -> Option<ModelChoice> {
    record
        .pinned_model
        .as_deref()
        .and_then(|text| registry.parse_pinned(text))
}

/// provider에 넘길 모델 이름.
pub(crate) fn pinned_model_name(registry: &Registry, record: &QueuedInput) -> Option<String> {
    match pinned_choice(registry, record) {
        Some(choice) => Some(choice.model),
        None => record.pinned_model.clone(),
    }
}

impl Engine {
    /// 채팅의 고정 모델을 저장하고 그 채팅에 붙은 모든 TUI에 알린다. 다음 입력부터 쓴다.
    ///
    /// # Errors
    /// 붙지 않은 채팅이면 `ChatNotAttached`, 저장 실패면 `Store`.
    pub(crate) async fn set_model(
        &mut self,
        client: ClientId,
        chat: ChatId,
        model: &ModelChoice,
    ) -> Result<(), EngineError> {
        self.attached_workdir(client, chat)?;
        self.store
            .set_chat_model(chat, &Registry::pinned_text(model))
            .await?;
        let notification = Notification::ModelPinned {
            chat,
            model: model.clone(),
        };
        self.rpc.broadcast(Some(chat), notification).await;
        Ok(())
    }

    /// 고정한 채팅에 붙을 때 그 모델을 알린다. 고정하지 않았으면 보내지 않는다.
    ///
    /// # Errors
    /// 없는 채팅이면 `Store(NotFound)`.
    pub(crate) async fn send_chat_model(
        &self,
        client: ClientId,
        chat: ChatId,
    ) -> Result<(), EngineError> {
        let pinned = self.store.chat_model(chat).await?;
        if let Some(model) = pinned
            .as_deref()
            .and_then(|text| self.registry.parse_pinned(text))
        {
            self.send(client, Notification::ModelPinned { chat, model })
                .await;
        }
        Ok(())
    }

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
        for provider in self
            .registry
            .ids()
            .into_iter()
            .filter(|provider| only.is_none_or(|only| only == *provider))
        {
            if !self.providers.contains_key(&(chat, provider))
                && !self.registry.is_installed(provider, &env)
            {
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

    /// 연결할 때 받아 둔 목록이 있으면 그것을 쓰고, 없으면 받아서 둔다.
    async fn list_provider_models(
        &mut self,
        provider: Provider,
        chat: ChatId,
        revision: SettingsRevision,
    ) -> Result<Vec<ModelInfo>, EngineError> {
        self.ensure_connected(provider, chat, revision).await?;
        if let Some(models) = self.flow.models.get(&(chat, provider)) {
            return Ok(models.clone());
        }
        let models = self.provider_mut(chat, provider)?.list_models().await?;
        self.flow.models.insert((chat, provider), models.clone());
        Ok(models)
    }

    /// 방금 연결한 provider의 모델 목록을 받아 둔다. 못 받으면 로그만 남기고 그 provider는 후보에 넣지 않는다.
    pub(crate) async fn remember_models(&mut self, provider: Provider, chat: ChatId) {
        let listed = match self.provider_mut(chat, provider) {
            Ok(client) => client.list_models().await,
            Err(error) => Err(error),
        };
        self.apply_models(provider, chat, listed);
    }

    /// 받은 모델 목록을 둔다. 못 받았으면 그 provider는 후보에 넣지 않는다.
    pub(crate) fn apply_models(
        &mut self,
        provider: Provider,
        chat: ChatId,
        listed: Result<Vec<ModelInfo>, ProviderError>,
    ) {
        match listed {
            Ok(models) => {
                self.flow.models.insert((chat, provider), models);
            }
            Err(error) => {
                self.flow.models.remove(&(chat, provider));
                tracing::warn!(error = %masked_chain(&self.masker, &EngineError::from(error)), "failed to list models");
            }
        }
    }

    /// router에 물을 허용 후보. 받아 둔 목록을 기본 순서로 `<provider>/<model>` 글로 만든다.
    pub(crate) fn model_candidates(&self, chat: ChatId) -> Vec<String> {
        self.registry
            .ids()
            .into_iter()
            .filter_map(|provider| self.flow.models.get(&(chat, provider)))
            .flatten()
            .map(|info| Registry::pinned_text(&info.choice))
            .collect()
    }
}
