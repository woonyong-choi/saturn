//! `saturn --resume`(채팅 id 없음)과 `--resume all`: 채팅 목록 선택 창에서 방향키로 골라 이어 연다.
//! 설계: docs/design/engine-lifecycle.md

use std::io::IsTerminal;

use saturn_protocol::ids::ChatId;
use saturn_protocol::rpc::{ChatListItem, Notification, Request};
use saturn_tui::client::EngineClient;
use saturn_tui::i18n::{self, Lang};

use crate::commands::call;

/// 목록을 받아 선택 창에서 골라 채팅 id를 돌려준다. `folder`가 `None`이면 모든 폴더.
///
/// # Errors
/// 채팅이 없거나, 창에서 `Esc`로 취소했거나, 연결이 끊기면 오류.
pub(crate) async fn pick(
    lang: Lang,
    client: &mut EngineClient,
    folder: Option<String>,
) -> anyhow::Result<ChatId> {
    let show_folder = folder.is_none();
    let mut answer = None;
    call(
        lang,
        client,
        Request::ListChats { folder },
        |notification| {
            if let Notification::ChatList { chats } = notification {
                answer = Some(chats);
            }
        },
    )
    .await?;
    let chats = answer.ok_or_else(|| anyhow::anyhow!(lang.tr(i18n::CLI_NO_CHAT_LIST_ANSWER)))?;
    pick_from(lang, chats, show_folder, saturn_tui::pick_chat)
}

/// 표준 입력이 터미널이 아니면 engine을 띄우기 전에 거절한다.
///
/// # Errors
/// 터미널이 아니면 오류.
pub(crate) fn ensure_terminal(lang: Lang) -> anyhow::Result<()> {
    check_terminal(lang, std::io::stdin().is_terminal())
}

fn check_terminal(lang: Lang, is_terminal: bool) -> anyhow::Result<()> {
    anyhow::ensure!(is_terminal, lang.tr(i18n::CLI_PICK_NEEDS_TERMINAL));
    Ok(())
}

/// 채팅이 없으면 창을 열지 않는다. `window`는 선택 창이고 취소하면 `None`.
fn pick_from(
    lang: Lang,
    chats: Vec<ChatListItem>,
    show_folder: bool,
    window: impl FnOnce(Lang, Vec<ChatListItem>, bool) -> Result<Option<ChatId>, saturn_tui::TuiError>,
) -> anyhow::Result<ChatId> {
    anyhow::ensure!(
        !chats.is_empty(),
        lang.tr(if show_folder {
            i18n::CLI_NO_CHAT_TO_RESUME
        } else {
            i18n::CLI_NO_CHAT_TO_CONTINUE
        })
    );
    let picked = window(lang, chats, show_folder)?;
    picked.ok_or_else(|| anyhow::anyhow!(lang.tr(i18n::CLI_PICK_CANCELLED)))
}

#[cfg(test)]
mod tests {
    use crate::testing::{FakeEngine, Reply};

    use super::*;

    fn item(chat: u64) -> ChatListItem {
        ChatListItem {
            chat: ChatId(chat),
            folder: "/work".to_owned(),
            name: None,
            last_active_ms: 0,
            preview: None,
            rows: None,
        }
    }

    #[test]
    fn resume_opens_the_chat_picked_in_the_window() {
        let chat = pick_from(
            Lang::En,
            vec![item(9), item(4)],
            true,
            |_, chats, show_folder| {
                assert_eq!(chats.len(), 2);
                assert!(show_folder);
                Ok(Some(ChatId(4)))
            },
        );

        assert_eq!(chat.unwrap(), ChatId(4));
    }

    #[test]
    fn resume_cancelled_in_the_window_opens_nothing_and_ends_with_an_error() {
        let error = pick_from(Lang::En, vec![item(9)], false, |_, _, _| Ok(None)).unwrap_err();

        assert_eq!(error.to_string(), "No chat was picked");
    }

    #[test]
    fn resume_with_no_chats_ends_with_guidance_and_opens_no_window() {
        let window = |_, _, _| panic!("window should not open without chats");
        let in_folder = pick_from(Lang::En, Vec::new(), false, window).unwrap_err();
        let in_all = pick_from(Lang::En, Vec::new(), true, window).unwrap_err();

        assert!(
            in_folder
                .to_string()
                .contains("No chat to continue in this folder")
        );
        assert!(in_all.to_string().starts_with("No chat to resume"));
    }

    #[test]
    fn resume_needs_a_terminal() {
        let error = check_terminal(Lang::En, false).unwrap_err();

        assert!(error.to_string().contains("No terminal to pick a chat in"));
        assert!(check_terminal(Lang::En, true).is_ok());
    }

    #[tokio::test]
    async fn pick_asks_the_engine_for_the_folder_or_every_folder() {
        for folder in [Some("/work".to_owned()), None] {
            let engine = FakeEngine::start(vec![Reply::with(vec![Notification::ChatList {
                chats: Vec::new(),
            }])]);
            let mut client = engine.client().await;

            let error = pick(Lang::En, &mut client, folder.clone())
                .await
                .unwrap_err();

            assert!(error.to_string().contains("No chat to"));
            let requests = engine.finish().await;
            assert!(matches!(
                requests.as_slice(),
                [Request::ListChats { folder: asked }] if *asked == folder
            ));
        }
    }
}
