//! `saturn --resume`(채팅 id 없음)과 `--resume all`: 채팅 목록에서 번호로 골라 이어 연다.
//! 설계: docs/design/engine-lifecycle.md

use std::io::{BufRead, IsTerminal, Write};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use saturn_protocol::ids::ChatId;
use saturn_protocol::rpc::{ChatListItem, Notification, Request};
use saturn_tui::client::EngineClient;
use saturn_tui::i18n::{self, Lang};

use crate::commands::call;

/// 목록 줄의 첫 입력 미리보기 글자 수. 초안 값.
const PREVIEW_CHARS: usize = 60;

const MINUTE_MS: u64 = 60_000;
const HOUR_MS: u64 = 60 * MINUTE_MS;
const DAY_MS: u64 = 24 * HOUR_MS;

/// 목록을 받아 골라 채팅 id를 돌려준다. `folder`가 `None`이면 모든 폴더.
///
/// # Errors
/// 채팅이 없거나, 번호를 고르지 않았거나, 연결이 끊기면 오류.
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
    let stdin = std::io::stdin();
    pick_from(
        lang,
        &chats,
        show_folder,
        now_ms(),
        stdin.lock(),
        &mut std::io::stderr(),
    )
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

// cost: time O(c), heap O(c), stack O(1), io c
// vars: c = 목록의 채팅 수
// basis: estimate
fn pick_from(
    lang: Lang,
    chats: &[ChatListItem],
    show_folder: bool,
    now_ms: u64,
    mut input: impl BufRead,
    out: &mut impl Write,
) -> anyhow::Result<ChatId> {
    anyhow::ensure!(
        !chats.is_empty(),
        lang.tr(if show_folder {
            i18n::CLI_NO_CHAT_TO_RESUME
        } else {
            i18n::CLI_NO_CHAT_TO_CONTINUE
        })
    );
    for (index, chat) in chats.iter().enumerate() {
        writeln!(out, "{}", row(lang, index + 1, chat, show_folder, now_ms))?;
    }
    write!(out, "{} ", lang.tr(i18n::CLI_PICK_PROMPT))?;
    out.flush()?;
    let mut line = String::new();
    input
        .read_line(&mut line)
        .context(lang.tr(i18n::CLI_PICK_CANCELLED))?;
    let text = line.trim();
    anyhow::ensure!(!text.is_empty(), lang.tr(i18n::CLI_PICK_CANCELLED));
    let picked = text
        .parse::<usize>()
        .ok()
        .and_then(|number| number.checked_sub(1))
        .and_then(|index| chats.get(index))
        .ok_or_else(|| anyhow::anyhow!(lang.tr(i18n::CLI_PICK_INVALID).replace("{text}", text)))?;
    Ok(picked.chat)
}

/// `번호. #채팅 id · 경과 · [폴더 ·] 첫 입력`
fn row(lang: Lang, number: usize, chat: &ChatListItem, show_folder: bool, now_ms: u64) -> String {
    let mut parts = vec![
        format!("{number}. #{}", chat.chat.0),
        age(lang, now_ms.saturating_sub(chat.last_active_ms)),
    ];
    if show_folder {
        parts.push(chat.folder.clone());
    }
    parts.push(preview(lang, chat.preview.as_deref()));
    parts.join(" · ")
}

fn age(lang: Lang, elapsed_ms: u64) -> String {
    let (phrase, count) = if elapsed_ms >= DAY_MS {
        (i18n::CLI_AGE_DAYS, elapsed_ms / DAY_MS)
    } else if elapsed_ms >= HOUR_MS {
        (i18n::CLI_AGE_HOURS, elapsed_ms / HOUR_MS)
    } else if elapsed_ms >= MINUTE_MS {
        (i18n::CLI_AGE_MINUTES, elapsed_ms / MINUTE_MS)
    } else {
        return lang.tr(i18n::CLI_AGE_NOW).to_owned();
    };
    lang.tr(phrase).replace("{n}", &count.to_string())
}

/// 첫 줄만, `PREVIEW_CHARS`자까지.
fn preview(lang: Lang, text: Option<&str>) -> String {
    let Some(first_line) = text.and_then(|text| text.lines().find(|line| !line.trim().is_empty()))
    else {
        return lang.tr(i18n::CLI_PICK_NO_INPUT).to_owned();
    };
    let first_line = first_line.trim();
    if first_line.chars().count() <= PREVIEW_CHARS {
        return first_line.to_owned();
    }
    let cut: String = first_line.chars().take(PREVIEW_CHARS).collect();
    format!("{cut}…")
}

fn now_ms() -> u64 {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use crate::testing::{FakeEngine, Reply};

    use super::*;

    const NOW: u64 = 10 * DAY_MS;

    fn item(chat: u64, folder: &str, last_active_ms: u64, preview: Option<&str>) -> ChatListItem {
        ChatListItem {
            chat: ChatId(chat),
            folder: folder.to_owned(),
            last_active_ms,
            preview: preview.map(str::to_owned),
        }
    }

    fn list() -> Vec<ChatListItem> {
        vec![
            item(
                9,
                "/work/a",
                NOW - 2 * HOUR_MS,
                Some("fix login\nsecond line"),
            ),
            item(4, "/work/b", NOW - 3 * DAY_MS, None),
        ]
    }

    #[test]
    fn resume_picker_lists_chats_and_returns_the_picked_chat() {
        let mut out = Vec::new();

        let chat = pick_from(Lang::En, &list(), false, NOW, "2\n".as_bytes(), &mut out);

        assert_eq!(chat.unwrap(), ChatId(4));
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "1. #9 · 2h ago · fix login\n2. #4 · 3d ago · (no input)\n\
             Enter the number of the chat to resume (Enter to cancel) "
        );
    }

    #[test]
    fn resume_picker_shows_the_folder_column_for_all_folders() {
        let mut out = Vec::new();

        pick_from(Lang::En, &list(), true, NOW, "1\n".as_bytes(), &mut out).unwrap();

        assert!(
            String::from_utf8(out)
                .unwrap()
                .starts_with("1. #9 · 2h ago · /work/a · fix login\n")
        );
    }

    #[test]
    fn resume_picker_without_a_choice_or_with_a_bad_number_fails() {
        for (answer, message) in [
            ("\n", "No chat was picked"),
            ("", "No chat was picked"),
            ("3\n", "Not in the list: 3"),
            ("0\n", "Not in the list: 0"),
            ("abc\n", "Not in the list: abc"),
        ] {
            let error = pick_from(
                Lang::En,
                &list(),
                false,
                NOW,
                answer.as_bytes(),
                &mut Vec::new(),
            )
            .unwrap_err();

            assert_eq!(error.to_string(), message, "{answer:?}");
        }
    }

    #[test]
    fn resume_picker_with_no_chats_ends_with_guidance_and_does_not_read() {
        let in_folder =
            pick_from(Lang::En, &[], false, 0, "1\n".as_bytes(), &mut Vec::new()).unwrap_err();
        let in_all =
            pick_from(Lang::En, &[], true, 0, "1\n".as_bytes(), &mut Vec::new()).unwrap_err();

        assert!(
            in_folder
                .to_string()
                .contains("No chat to continue in this folder")
        );
        assert!(in_all.to_string().starts_with("No chat to resume"));
    }

    #[test]
    fn resume_picker_needs_a_terminal() {
        let error = check_terminal(Lang::En, false).unwrap_err();

        assert!(error.to_string().contains("No terminal to pick a chat in"));
        assert!(check_terminal(Lang::En, true).is_ok());
    }

    #[test]
    fn long_previews_are_cut_to_the_first_line() {
        let long = "가".repeat(PREVIEW_CHARS + 5);

        let text = preview(Lang::En, Some(&long));

        assert_eq!(text.chars().count(), PREVIEW_CHARS + 1);
        assert!(text.ends_with('…'));
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
