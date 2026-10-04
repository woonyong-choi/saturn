//! provider 명령 목록 전달: 어댑터가 알린 목록을 TUI의 `/`와 `$` 팝업이 쓰는 `Commands` 알림으로 보낸다.
//! 설계: docs/design/extensions.md#기능-목록

use saturn_core::providers::ProviderCommand;
use saturn_protocol::ids::{ChatId, Provider};
use saturn_protocol::rpc::{CommandInfo, Notification};

use crate::Engine;
use crate::rpc::ClientId;

impl Engine {
    /// 연결이 알린 목록을 기억하고 그 채팅에 붙은 모든 TUI에 보낸다.
    pub(crate) async fn on_commands(
        &mut self,
        chat: ChatId,
        provider: Provider,
        commands: Vec<ProviderCommand>,
    ) {
        let commands: Vec<CommandInfo> = commands
            .into_iter()
            .map(|command| CommandInfo {
                name: command.name,
                description: command.description,
                is_skill: command.is_skill,
            })
            .collect();
        self.flow
            .commands
            .insert((chat, provider), commands.clone());
        self.rpc
            .broadcast(Some(chat), Notification::Commands { provider, commands })
            .await;
    }

    /// 붙을 때 그 채팅의 연결이 알린 목록을 보낸다. 알린 적 없는 연결은 보내지 않는다.
    pub(crate) async fn send_chat_commands(&self, client: ClientId, chat: ChatId) {
        for provider in self.registry.ids() {
            if let Some(commands) = self.flow.commands.get(&(chat, provider)) {
                let notification = Notification::Commands {
                    provider,
                    commands: commands.clone(),
                };
                self.send(client, notification).await;
            }
        }
    }
}
