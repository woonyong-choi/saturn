//! 설정 파일 감시: 붙은 채팅의 사용자·폴더 설정 파일이 바뀌면 다음 입력을 기다리지 않고 적용한다.
//! 설계: docs/design/settings.md#설정-변경-감지

use std::path::Path;
use std::time::Duration;

use saturn_protocol::ids::ChatId;

use crate::Engine;
use crate::rpc::ClientId;
use crate::settings::FileFingerprints;

/// 설정 파일을 다시 확인하는 주기. 편집기의 저장이 한두 번의 쓰기로 끝나는 시간보다 짧고, 채팅마다 작은 파일 두세 개를
/// 읽는 비용이 무시할 만해서 짧게 둔다. 바뀐 지문이 이 주기 두 번 연달아 같을 때만 적용하므로 저장 뒤 적용까지 1초 안팎이다.
pub(crate) const SETTINGS_WATCH_TICK: Duration = Duration::from_millis(500);

/// 채팅마다 마지막으로 본 바뀐 지문과, 그 지문을 이미 적용해 봤는지.
pub(crate) type WatchedSettings =
    std::collections::HashMap<(ChatId, Vec<String>), (FileFingerprints, bool)>;

impl Engine {
    /// 붙은 채팅마다 설정 파일을 확인해 바뀌었으면 적용하고 연결에 반영한다. 편집기가 파일을 쓰는 도중에 읽어 반쯤 쓴
    /// 내용을 적용하지 않도록, 바뀐 지문을 처음 본 확인에서는 기록만 하고 다음 확인에서도 같을 때 적용한다. 적용이 실패해도
    /// 같은 지문으로는 다시 시도하지 않는다. 내용 검사 실패는 적용 쪽이 이전 설정 유지와 경고로 처리한다.
    pub(crate) async fn watch_settings(&mut self) {
        let mut attached: Vec<(ChatId, Vec<String>, Vec<ClientId>)> = Vec::new();
        for (client, attachment) in &self.attachments {
            let run = attachment.run_layer();
            match attached
                .iter_mut()
                .find(|(chat, scope, _)| *chat == attachment.chat && *scope == run)
            {
                Some((_, _, clients)) => clients.push(*client),
                None => attached.push((attachment.chat, run, vec![*client])),
            }
        }
        self.flow.watched_settings.retain(|(chat, run), _| {
            attached
                .iter()
                .any(|(open, scope, _)| open == chat && scope == run)
        });
        for (chat, run, clients) in attached {
            self.watch_chat_settings(chat, &run, &clients).await;
        }
    }

    async fn watch_chat_settings(&mut self, chat: ChatId, run: &[String], clients: &[ClientId]) {
        let Some(workdir) = self.chat_env(chat).map(|env| env.workdir().to_path_buf()) else {
            return;
        };
        if !self.settled_change(chat, run, &workdir).await {
            return;
        }
        match self.apply_changed_settings(clients, chat, &workdir).await {
            Ok(revision) => self.sync_provider_settings(chat, revision).await,
            Err(error) => {
                tracing::warn!(error = %self.failure_line(&error), "changed settings not applied");
            }
        }
    }

    /// 바뀐 지문이 두 번 연달아 같고 아직 적용해 보지 않았으면 참.
    async fn settled_change(&mut self, chat: ChatId, run: &[String], workdir: &Path) -> bool {
        let observed = match self.settings.observe(Some(chat), workdir, run).await {
            Ok(observed) => observed,
            Err(error) => {
                tracing::debug!(error = %self.failure_line(&error), "settings files not read");
                return false;
            }
        };
        let Some(now) = observed else {
            self.flow.watched_settings.remove(&(chat, run.to_vec()));
            return false;
        };
        let key = (chat, run.to_vec());
        match self.flow.watched_settings.get_mut(&key) {
            Some((seen, tried)) if *seen == now => !std::mem::replace(tried, true),
            _ => {
                self.flow.watched_settings.insert(key, (now, false));
                false
            }
        }
    }
}
