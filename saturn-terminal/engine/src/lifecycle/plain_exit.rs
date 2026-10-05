//! plain 출력의 종료: 실제 engine과 가짜 provider에 소켓으로 붙어 입력 한 줄과 EOF 뒤 끝나는 때와 종료 결과를 본다.
//! 설계: docs/design/tui.md#단순-방식

use std::io::Write;
use std::sync::{Arc, Mutex};

use saturn_core::providers::ProviderError;
use saturn_protocol::ids::AgentId;
use saturn_tui::client::EngineClient;
use saturn_tui::i18n::Lang;
use saturn_tui::{RunOptions, TuiError, run_plain_on};
use tokio::io::BufReader;

use super::support::{Flow, idle_reply, running_reply};
use super::*;
use crate::providers::test_support::{Call, FakeProvider};

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Captured {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

/// provider가 받은 턴마다 끝났다고 알린다. 열린 에이전트와 받은 턴 수는 가짜의 호출 기록으로 안다.
/// `wait_for`가 있으면 그 글이 출력에 나온 뒤에만 알린다. 뒤 입력이 실행 중인 작업을 보게 하려는 것이다.
async fn complete_turns(fake: FakeProvider, out: Captured, wait_for: Option<&str>) {
    let mut completed = 0;
    loop {
        let calls = fake.calls();
        let agent: Option<AgentId> = calls.iter().rev().find_map(|call| match call {
            Call::Open { agent, .. } => Some(*agent),
            _ => None,
        });
        let sent = calls
            .iter()
            .filter(|call| matches!(call, Call::SendTurn { .. }))
            .count();
        let is_ready = wait_for.is_none_or(|text| out.text().contains(text));
        if let Some(agent) = agent
            && completed < sent
            && is_ready
        {
            completed += 1;
            fake.emit(support::text(agent, "done\n"));
            fake.emit(support::turn_completed(agent));
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// 실제 engine에 plain으로 붙어 `input`을 보내고 끝난 결과와 출력을 받는다. 끝나지 않으면 `None`.
async fn run_plain(
    flow: &mut Flow,
    input: &'static str,
    wait_for: Option<&'static str>,
) -> (Option<Result<(), TuiError>>, String) {
    let socket = flow.fixture.socket();
    let options = RunOptions {
        chat: Some(flow.chat),
        lang: Some(Lang::Ko),
        workdir: flow.fixture.workdir.clone(),
        overrides: Vec::new(),
        add_dirs: Vec::new(),
        history: flow.fixture.root.path().join("history"),
        child: None,
        plain: Some(true),
    };
    let out = Captured::default();
    let fake = flow.fake.clone();
    let written = out.clone();
    let shown = out.clone();
    let result = drive(&mut flow.engine, async move {
        let mut client = EngineClient::connect(&socket).await.unwrap();
        let session = run_plain_on(
            &mut client,
            options,
            BufReader::new(input.as_bytes()),
            written,
        );
        tokio::select! {
            result = session => Some(result),
            () = complete_turns(fake, shown, wait_for) => None,
            () = tokio::time::sleep(Duration::from_secs(3)) => None,
        }
    })
    .await;
    (result, out.text())
}

#[tokio::test]
async fn plain_ends_by_the_result_of_its_only_task() {
    type Answers = Vec<Result<(), ProviderError>>;
    // (사례, provider가 턴을 받은 결과 목록, 성공으로 끝나야 하는가)
    let cases: [(&str, Answers, bool); 3] = [
        ("the only task is done", Vec::new(), true),
        (
            "the provider refuses the turn",
            (0..8)
                .map(|_| {
                    Err(ProviderError::NotSent {
                        reason: "provider refused".to_owned(),
                    })
                })
                .collect(),
            false,
        ),
        (
            "the turn result is unknown",
            vec![Err(ProviderError::Unknown)],
            false,
        ),
    ];

    for (name, answers, succeeds) in cases {
        let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
        if !answers.is_empty() {
            flow.fake.answer_send(answers);
        }

        let (result, text) = run_plain(&mut flow, "hello\n", None).await;

        if succeeds {
            assert!(
                matches!(result, Some(Ok(()))),
                "{name}: plain should end with success, got {result:?}\n{text}"
            );
            assert!(text.contains("done"), "{name}: {text}");
        } else {
            assert!(
                matches!(result, Some(Err(TuiError::TaskFailed))),
                "{name}: plain should end with a task failure, got {result:?}\n{text}"
            );
        }
    }
}

#[tokio::test]
async fn plain_ends_after_the_task_that_took_the_second_input() {
    // (사례, router 판단, 기대 SendTurn 수, 기대 Steer 수)
    let cases = [
        ("queued input", "continues", "queue", Some(2), None),
        ("steered input", "refines", "steer", None, Some(1)),
    ];

    for (name, relation, action, turns, steers) in cases {
        let mut flow = Flow::new(vec![
            idle_reply(0.95),
            running_reply(0.95, relation, action),
            running_reply(0.95, relation, action),
        ])
        .await;
        if action == "steer" {
            flow.fake.verify_steer();
        }

        let (result, text) = run_plain(&mut flow, "one\ntwo\n", Some("two")).await;

        assert!(
            matches!(result, Some(Ok(()))),
            "{name}: plain should end after its tasks, got {result:?}\n{text}"
        );
        let calls = flow.fake.calls();
        let count = |wanted: fn(&Call) -> bool| calls.iter().filter(|call| wanted(call)).count();
        if let Some(turns) = turns {
            assert_eq!(
                count(|call| matches!(call, Call::SendTurn { .. })),
                turns,
                "{name}"
            );
        }
        if let Some(steers) = steers {
            assert_eq!(
                count(|call| matches!(call, Call::Steer { .. })),
                steers,
                "{name}"
            );
        }
    }
}
