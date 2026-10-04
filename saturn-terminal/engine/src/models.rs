//! 고를 수 있는 모델 목록과 입력에 고정한 모델 읽기.
//! 설계: docs/design/providers-and-sessions.md#모델-고르기

use std::collections::HashMap;

use saturn_core::providers::ProviderError;
use saturn_core::queue::QueuedInput;
use saturn_protocol::ids::{ChatId, Provider, SettingsRevision};
use saturn_protocol::rpc::{ModelChoice, ModelInfo, ModelMode, Notification, QueryResult};

use crate::calls::{CallKind, PendingCall, Responder};
use crate::launch::ConnectionSeed;
use crate::providers::{Connected, Registry, spawn_connect};
use crate::rpc::ClientId;
use crate::settings::{Settings, SettingsError};
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

/// 모델 목록을 모으는 `/model` 요청. 모든 조회가 돌아오면 목록을 응답의 `result`로 돌려준다.
#[derive(Debug)]
pub(crate) struct ModelsQuery {
    responder: Responder,
    /// 목록을 받을 provider. 알리는 순서다.
    providers: Vec<Provider>,
    listed: HashMap<Provider, Result<Vec<ModelInfo>, EngineError>>,
    /// 맡겨 두고 결과를 기다리는 조회 수.
    waiting: usize,
}

/// 입력이 접수 때 고정한 설정 번호의 모델 선택 값.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModelPlan {
    pub(crate) mode: ModelMode,
    /// 기본 모델의 `<provider>/<model>` 글. 고르지 않았거나 등록하지 않은 provider면 `None`.
    pub(crate) default: Option<String>,
}

impl ModelPlan {
    pub(crate) fn from_settings(settings: &Settings, registry: &Registry) -> Self {
        Self {
            mode: settings.model_mode(),
            default: settings
                .model_default()
                .filter(|text| registry.parse_pinned(text).is_some())
                .map(str::to_owned),
        }
    }
}

impl Engine {
    /// 설정 번호의 모델 선택 값.
    ///
    /// # Errors
    /// 없는 번호면 `Settings`.
    pub(crate) async fn model_plan(
        &self,
        revision: SettingsRevision,
    ) -> Result<ModelPlan, EngineError> {
        let settings = self.settings.at(&self.store, revision).await?;
        Ok(ModelPlan::from_settings(&settings, &self.registry))
    }

    /// 기본 모델을 사용자 설정 파일에 저장하고 붙은 TUI에 알린다. 다음 입력부터 쓴다.
    ///
    /// # Errors
    /// 붙지 않은 채팅이면 `ChatNotAttached`, 파일 저장이나 병합 실패면 `Settings`.
    pub(crate) async fn set_default_model(
        &mut self,
        client: ClientId,
        chat: ChatId,
        model: &ModelChoice,
    ) -> Result<(), EngineError> {
        let workdir = self.attached_workdir(client, chat)?;
        let value = toml_edit::Value::from(Registry::pinned_text(model)).to_string();
        self.settings
            .set_user_value("model.default", &value)
            .await?;
        self.apply_changed_settings(&[client], chat, &workdir)
            .await?;
        Ok(())
    }

    /// 모델 선택 방식을 사용자 설정 파일에 저장하고 붙은 TUI에 알린다. 다음 입력부터 쓴다.
    ///
    /// # Errors
    /// `set_default_model`과 같다.
    pub(crate) async fn set_model_mode(
        &mut self,
        client: ClientId,
        chat: ChatId,
        mode: ModelMode,
    ) -> Result<(), EngineError> {
        let workdir = self.attached_workdir(client, chat)?;
        let value = match mode {
            ModelMode::Auto => "\"auto\"",
            ModelMode::Manual => "\"manual\"",
        };
        self.settings.set_user_value("model.mode", value).await?;
        self.apply_changed_settings(&[client], chat, &workdir)
            .await?;
        Ok(())
    }

    /// 채팅에 쓰는 설정의 기본 모델과 방식을 붙은 TUI에 알린다. 마지막으로 알린 값과 같으면 보내지 않는다.
    ///
    /// # Errors
    /// 없는 설정 번호면 `Settings`.
    pub(crate) async fn announce_model_settings(
        &mut self,
        chat: ChatId,
        revision: SettingsRevision,
    ) -> Result<(), EngineError> {
        let plan = self.model_plan(revision).await?;
        if self.flow.model_shown.get(&chat) == Some(&plan) {
            return Ok(());
        }
        self.flow.model_shown.insert(chat, plan.clone());
        let notification = self.model_settings_notification(chat, &plan);
        self.rpc.broadcast(Some(chat), notification).await;
        Ok(())
    }

    /// 채팅에 붙을 때 지금 값을 그 TUI에 알린다.
    ///
    /// # Errors
    /// 설정이 없으면 `Settings`.
    pub(crate) async fn send_model_settings(
        &mut self,
        client: ClientId,
        chat: ChatId,
    ) -> Result<(), EngineError> {
        let revision = self
            .settings
            .current()
            .ok_or(SettingsError::NoPreviousRevision)?;
        let plan = self.model_plan(revision).await?;
        self.flow.model_shown.insert(chat, plan.clone());
        let notification = self.model_settings_notification(chat, &plan);
        self.send(client, notification).await;
        Ok(())
    }

    fn model_settings_notification(&self, chat: ChatId, plan: &ModelPlan) -> Notification {
        Notification::ModelSettings {
            chat,
            default: plan
                .default
                .as_deref()
                .and_then(|text| self.registry.parse_pinned(text)),
            mode: plan.mode,
        }
    }

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

    /// 고정한 채팅에 붙을 때 그 모델을 알린다. 고정하지 않았으면 보내지 않는다. 이어서 기본 모델과 선택 방식을 알린다.
    ///
    /// # Errors
    /// 없는 채팅이면 `Store(NotFound)`.
    pub(crate) async fn send_chat_model(
        &mut self,
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
        self.send_model_settings(client, chat).await
    }

    // cost: time O(m), heap O(m), stack O(1), io p
    // vars: m = 모델 수, p = provider 수
    // basis: estimate
    /// 설치된 provider마다 모델 목록을 받아 `Models` 결과로 돌려준다. 연결과 조회는 연결 작업이 하고, 모든 목록이
    /// 모이면 `responder`에 응답한다. 목록을 못 받은 provider는 건너뛰고 로그만 남긴다. 모든 provider가 실패해도
    /// 창이 끝없이 기다리지 않도록 빈 목록을 결과로 돌려준다.
    ///
    /// 응답 오류: 붙지 않은 채팅이면 `ChatNotAttached`, 설정이 없으면 `Settings`.
    pub(crate) async fn send_models(
        &mut self,
        client: ClientId,
        chat: ChatId,
        only: Option<Provider>,
        responder: Responder,
    ) {
        let (revision, providers) = match self.models_wanted(client, chat, only) {
            Ok(wanted) => wanted,
            Err(error) => return self.respond(responder, Err(error)).await,
        };
        let query = self.flow.issue_call();
        self.flow.model_queries.insert(
            query,
            ModelsQuery {
                responder,
                providers: providers.clone(),
                listed: HashMap::new(),
                waiting: 0,
            },
        );
        for provider in providers {
            self.request_models(query, chat, provider, revision).await;
        }
        self.finish_models(query).await;
    }

    /// 목록을 받을 provider를 기본 순서로. 연결이 없고 설치되지도 않은 provider는 뺀다.
    fn models_wanted(
        &self,
        client: ClientId,
        chat: ChatId,
        only: Option<Provider>,
    ) -> Result<(SettingsRevision, Vec<Provider>), EngineError> {
        self.attached_workdir(client, chat)?;
        let revision = self
            .settings
            .current()
            .ok_or(SettingsError::NoPreviousRevision)?;
        let env = self
            .chat_env(chat)
            .map(crate::chat_env::ChatEnv::provider_env)
            .unwrap_or_default();
        let providers = self
            .registry
            .ids()
            .into_iter()
            .filter(|provider| only.is_none_or(|only| only == *provider))
            .filter(|provider| {
                self.providers.contains_key(&(chat, *provider))
                    || self.registry.is_installed(*provider, &env)
            })
            .collect();
        Ok((revision, providers))
    }

    /// 연결할 때 받아 둔 목록이 있으면 그것을 쓰고, 없으면 연결 작업에 조회를 맡긴다. 연결이 없으면 연결부터 맡긴다.
    async fn request_models(
        &mut self,
        query: u64,
        chat: ChatId,
        provider: Provider,
        revision: SettingsRevision,
    ) {
        if self.providers.contains_key(&(chat, provider)) {
            if let Some(models) = self.flow.models.get(&(chat, provider)) {
                let models = models.clone();
                self.note_models(query, provider, Ok(models));
                return;
            }
            self.start_call(
                (chat, provider),
                CallKind::ModelList { query },
                |connection, tag| connection.list_models_call(tag),
            );
            self.wait_for_models(query);
            return;
        }
        let started = self
            .start_models_connect(query, chat, provider, revision)
            .await;
        match started {
            Ok(()) => self.wait_for_models(query),
            Err(error) => self.note_models(query, provider, Err(error)),
        }
    }

    /// 연결을 맺고 모델 목록까지 받는 작업을 맡긴다.
    async fn start_models_connect(
        &mut self,
        query: u64,
        chat: ChatId,
        provider: Provider,
        revision: SettingsRevision,
    ) -> Result<(), EngineError> {
        let launch = self.launch_spec(provider, chat, revision).await?;
        let seed = ConnectionSeed::of(&launch);
        let adapter = self
            .registry
            .get(provider)
            .cloned()
            .ok_or(ProviderError::ConnectionLost)?;
        let tag = self.flow.issue_call();
        self.flow.calls.insert(
            tag,
            PendingCall {
                chat,
                provider,
                kind: CallKind::ModelConnect { query, seed },
            },
        );
        spawn_connect(
            chat,
            launch,
            adapter,
            self.supervisor.clone(),
            self.flow.provider_tx.clone(),
            Some(tag),
        );
        Ok(())
    }

    fn wait_for_models(&mut self, query: u64) {
        if let Some(entry) = self.flow.model_queries.get_mut(&query) {
            entry.waiting += 1;
        }
    }

    fn note_models(
        &mut self,
        query: u64,
        provider: Provider,
        listed: Result<Vec<ModelInfo>, EngineError>,
    ) {
        if let Some(entry) = self.flow.model_queries.get_mut(&query) {
            entry.listed.insert(provider, listed);
        }
    }

    /// 맡긴 조회의 결과. 받았으면 연결의 목록으로 두어 다음 요청과 후보가 쓴다.
    pub(crate) async fn on_models_listed(
        &mut self,
        query: u64,
        (chat, provider): (ChatId, Provider),
        listed: Result<Vec<ModelInfo>, ProviderError>,
    ) {
        let recorded = self.record_models(provider, chat, listed);
        self.note_models(query, provider, recorded.map_err(EngineError::from));
        self.models_arrived(query).await;
    }

    /// 맡긴 연결의 결과. 그사이 다른 요청이 같은 연결을 맺었으면 그쪽을 쓰고 이 연결은 닫는다.
    pub(crate) async fn on_models_connected(
        &mut self,
        query: u64,
        (chat, provider): (ChatId, Provider),
        seed: ConnectionSeed,
        connected: Result<Box<Connected>, ProviderError>,
    ) {
        let listed = match connected {
            Ok(connected) => {
                let Connected { connection, models } = *connected;
                if self.providers.contains_key(&(chat, provider)) {
                    models
                } else {
                    self.attach_connection(chat, connection, seed);
                    self.record_models(provider, chat, models)
                }
            }
            Err(error) => Err(error),
        };
        self.note_models(query, provider, listed.map_err(EngineError::from));
        self.models_arrived(query).await;
    }

    async fn models_arrived(&mut self, query: u64) {
        if let Some(entry) = self.flow.model_queries.get_mut(&query) {
            entry.waiting = entry.waiting.saturating_sub(1);
        }
        self.finish_models(query).await;
    }

    /// 기다리는 조회가 없으면 모은 목록을 알리고 응답한다. 창이 끝없이 기다리지 않도록 실패해도 빈 목록은 보낸다.
    async fn finish_models(&mut self, query: u64) {
        if self
            .flow
            .model_queries
            .get(&query)
            .is_none_or(|entry| entry.waiting > 0)
        {
            return;
        }
        let Some(ModelsQuery {
            responder,
            providers,
            mut listed,
            ..
        }) = self.flow.model_queries.remove(&query)
        else {
            return;
        };
        let mut models: Vec<ModelInfo> = Vec::new();
        for provider in providers {
            match listed.remove(&provider) {
                Some(Ok(list)) => models.extend(list),
                Some(Err(error)) => {
                    tracing::warn!(error = %masked_chain(&self.masker, &error), "failed to list models");
                }
                None => {}
            }
        }
        self.respond_with(responder, Ok(Some(QueryResult::Models { models })))
            .await;
    }

    /// 받은 모델 목록을 둔다. 못 받았으면 그 provider는 후보에 넣지 않는다.
    pub(crate) fn apply_models(
        &mut self,
        provider: Provider,
        chat: ChatId,
        listed: Result<Vec<ModelInfo>, ProviderError>,
    ) {
        if let Err(error) = self.record_models(provider, chat, listed) {
            tracing::warn!(error = %masked_chain(&self.masker, &EngineError::from(error)), "failed to list models");
        }
    }

    /// `apply_models`와 같고 둔 목록이나 못 받은 이유를 돌려준다.
    fn record_models(
        &mut self,
        provider: Provider,
        chat: ChatId,
        listed: Result<Vec<ModelInfo>, ProviderError>,
    ) -> Result<Vec<ModelInfo>, ProviderError> {
        match listed {
            Ok(models) => {
                self.flow.models.insert((chat, provider), models.clone());
                Ok(models)
            }
            Err(error) => {
                self.flow.models.remove(&(chat, provider));
                Err(error)
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
