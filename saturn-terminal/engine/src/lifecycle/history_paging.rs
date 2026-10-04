//! 위로 스크롤한 이전 기록 요청 테스트: 받은 묶음의 `oldest`보다 앞을 겹침 없이 잇고, 끝에 닿으면 알리며, TUI마다 따로 잇는다(#110).

use std::time::Duration;

use saturn_core::queue::Permission;
use saturn_protocol::ids::{AgentId, ChatId, LedgerSeq, SessionId, SettingsRevision, TaskId};
use saturn_protocol::rpc::{Notification, Request};
use saturn_protocol::state::EffectScope;

use super::*;
use crate::store::{NewInput, NewRun, RunEnd};

/// 입력 `count`개와 끝난 실행 `count`개가 번갈아 쌓인 채팅. 단위는 모두 `2 * count`개다.
/// 기록 시각이 밀리초라 단위마다 시각이 다르도록 쉬어 가며 쌓는다.
async fn chat_with_exchanges(engine: &Engine, workdir: &Path, count: u64) -> ChatId {
    let store = &engine.store;
    let chat = store.create_chat(workdir.to_path_buf()).await.unwrap();
    for task in 1..=count {
        let input = store
            .accept_input(&NewInput {
                chat,
                text: format!("question {task}"),
                settings: SettingsRevision(1),
                permission: Permission::Write,
                workdir: workdir.to_path_buf(),
                pinned_model: None,
                skip_relation: false,
            })
            .await
            .unwrap();
        std::thread::sleep(Duration::from_millis(3));
        let run = store
            .start_run(&NewRun {
                input: Some(input),
                task: TaskId(task),
                agent: AgentId(1),
                session: SessionId(7),
                provider: crate::providers::test_support::CODEX,
                effect_scope: EffectScope::NetworkPossible,
            })
            .await
            .unwrap();
        store.finish_run(run, RunEnd::Completed).await.unwrap();
        std::thread::sleep(Duration::from_millis(3));
    }
    chat
}

fn attach_request(chat: ChatId, workdir: &Path) -> Request {
    Request::Attach {
        chat: Some(chat),
        workdir: workdir.display().to_string(),
        env: Vec::new(),
        overrides: Vec::new(),
        add_dirs: Vec::new(),
    }
}

struct Chunk {
    inputs: Vec<String>,
    oldest: Option<LedgerSeq>,
    has_more: bool,
}

fn chunk_of(received: &[Notification]) -> Chunk {
    let Some(Notification::HistoryChunk {
        entries,
        oldest,
        has_more,
        ..
    }) = received
        .iter()
        .find(|n| matches!(n, Notification::HistoryChunk { .. }))
    else {
        panic!("expected HistoryChunk, got {received:?}");
    };
    Chunk {
        inputs: entries
            .iter()
            .filter_map(|entry| match entry {
                Notification::InputChanged { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect(),
        oldest: *oldest,
        has_more: *has_more,
    }
}

async fn load(
    client: &mut Client,
    id: u64,
    chat: ChatId,
    before: Option<LedgerSeq>,
    limit: u32,
) -> Chunk {
    chunk_of(
        &client
            .attach(
                id,
                Request::LoadHistory {
                    chat,
                    before,
                    limit,
                },
            )
            .await,
    )
}

#[tokio::test]
async fn load_history_continues_before_the_oldest_of_the_previous_chunk_without_overlap() {
    let fixture = Fixture::new();
    let mut engine = fixture.ready().await;
    let chat = chat_with_exchanges(&engine, &fixture.workdir, 5).await;
    let mut client = Client::connect(&fixture.socket()).await;

    let chunks = drive(&mut engine, async {
        client
            .attach(1, attach_request(chat, &fixture.workdir))
            .await;
        let mut chunks = vec![load(&mut client, 2, chat, None, 4).await];
        while chunks.last().unwrap().has_more && chunks.len() < 6 {
            let before = chunks.last().unwrap().oldest;
            let id = 2 + u64::try_from(chunks.len()).unwrap();
            chunks.push(load(&mut client, id, chat, before, 4).await);
        }
        chunks
    })
    .await;

    // 단위 10개를 4개씩: 입력 2개씩 담은 묶음 둘과 남은 한 묶음
    let texts: Vec<Vec<&str>> = chunks
        .iter()
        .map(|chunk| chunk.inputs.iter().map(String::as_str).collect())
        .collect();
    assert_eq!(
        texts,
        vec![
            vec!["question 4", "question 5"],
            vec!["question 2", "question 3"],
            vec!["question 1"],
        ]
    );
    assert!(!chunks[2].has_more);
}

#[tokio::test]
async fn load_history_at_the_start_reports_no_more_and_no_oldest_when_nothing_is_left() {
    let fixture = Fixture::new();
    let mut engine = fixture.ready().await;
    let chat = chat_with_exchanges(&engine, &fixture.workdir, 2).await;
    let mut client = Client::connect(&fixture.socket()).await;

    let (all, past_start) = drive(&mut engine, async {
        client
            .attach(1, attach_request(chat, &fixture.workdir))
            .await;
        let all = load(&mut client, 2, chat, None, 50).await;
        let past_start = load(&mut client, 3, chat, all.oldest, 50).await;
        (all, past_start)
    })
    .await;

    assert!(!all.has_more);
    assert!(all.oldest.is_some());
    assert!(past_start.inputs.is_empty());
    assert_eq!(past_start.oldest, None);
    assert!(!past_start.has_more);
}

#[tokio::test]
async fn two_clients_on_one_chat_each_continue_from_their_own_oldest() {
    let fixture = Fixture::new();
    let mut engine = fixture.ready().await;
    let chat = chat_with_exchanges(&engine, &fixture.workdir, 5).await;
    let mut first = Client::connect(&fixture.socket()).await;
    let mut second = Client::connect(&fixture.socket()).await;

    let (first_next, second_next) = drive(&mut engine, async {
        first
            .attach(1, attach_request(chat, &fixture.workdir))
            .await;
        second
            .attach(1, attach_request(chat, &fixture.workdir))
            .await;
        let first_now = load(&mut first, 2, chat, None, 4).await;
        let first_next = load(&mut first, 3, chat, first_now.oldest, 4).await;
        // 첫 화면이 더 불러온 뒤에 둘째 화면은 처음 묶음에서 이어 간다
        let second_now = load(&mut second, 2, chat, None, 2).await;
        let second_next = load(&mut second, 3, chat, second_now.oldest, 4).await;
        (first_next, second_next)
    })
    .await;

    assert_eq!(first_next.inputs, vec!["question 2", "question 3"]);
    assert_eq!(second_next.inputs, vec!["question 3", "question 4"]);
}
