//! 하위 접속 부하 시험(#33). 가짜 provider로 하위 작업 N개가 동시에 붙고 끝나는 동안 engine 요청 응답 시간과 메모리를 잰다.
//! 오래 걸려 `#[ignore]`다. 실행: `cargo test -p saturn-engine --lib child_load -- --ignored --nocapture --test-threads=1`
//! 환경 변수 `CHILD_LOAD_NO_PROCESSES`가 있으면 프로세스를 띄우지 않는다.
//! 하위 채팅마다 `/bin/sleep` 프로세스 하나를 `Supervisor`로 띄워 provider 프로세스 감시 비용을 함께 잰다(실제 provider는 쓰지 않는다).

use std::io::Write;
use std::time::{Duration, Instant};

use saturn_protocol::envelope::ServerMessage;
use saturn_protocol::ids::ChatId;
use saturn_protocol::rpc::Request;

use super::child_passes::{attach_child, chat_of, family, give_provider};
use super::*;
use crate::processes::ProcessSpec;

/// 응답을 기다리는 시간. 동시 요청 수백 개가 한 번에 몰리는 구간을 재야 하므로 평소 시험보다 길다.
const PATIENCE: Duration = Duration::from_secs(120);

fn rss_kib() -> u64 {
    let pid = std::process::id().to_string();
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid])
        .output()
        .expect("ps should run");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(0)
}

fn percentile(sorted: &[Duration], percent: usize) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    sorted[(sorted.len() * percent / 100).min(sorted.len() - 1)]
}

/// `Version` 요청을 20ms 간격으로 보내 응답 시간을 모은다. `stop`이 오면 끝난다.
fn spawn_probe(
    socket: std::path::PathBuf,
    mut stop: tokio::sync::oneshot::Receiver<()>,
) -> tokio::task::JoinHandle<Vec<Duration>> {
    tokio::spawn(async move {
        let mut client = Client::connect(&socket).await;
        let mut samples = Vec::new();
        let mut id = 0;
        while stop.try_recv().is_err() {
            id += 1;
            let started = Instant::now();
            client.send(id, Request::Version).await;
            loop {
                if let ServerMessage::Response(_) = client.recv().await {
                    break;
                }
            }
            samples.push(started.elapsed());
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        samples
    })
}

async fn measure(count: usize) {
    let with_processes = std::env::var_os("CHILD_LOAD_NO_PROCESSES").is_none();
    let config = "child.max_concurrent = 1000\nchild.max_total = 1000\nchild.max_depth = 2\n";
    let mut family = family(config, count).await;
    let socket = family.flow.fixture.socket();
    let pass = family.pass.clone();
    // 채팅 번호는 순서대로 매겨지므로 하위 채팅이 생기기 전에 가짜 provider를 미리 붙여, 전 과정을 소켓 요청으로만 돌린다
    let first = family.flow.chat.0 + 1;
    for chat in first..first + count as u64 {
        give_provider(&mut family.flow, ChatId(chat));
    }
    let workdir = family.flow.fixture.workdir.clone();
    let supervisor = family.flow.engine.supervisor.clone();
    let groups: Vec<_> = (0..if with_processes { count } else { 0 })
        .map(|_| {
            supervisor
                .spawn(ProcessSpec {
                    program: "/bin/sleep".into(),
                    args: vec!["600".to_owned()],
                    workdir: workdir.clone(),
                    env: vec![("PATH".into(), "/bin:/usr/bin".into())],
                })
                .expect("sleep should start")
                .group
        })
        .collect();
    let before = rss_kib();
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    let probe = spawn_probe(socket.clone(), stop_rx);

    // 하위 접속 N개가 동시에 붙어 입력을 보내고, 실행 중인 채로 한꺼번에 끊긴다
    let tasks: Vec<_> = (0..count)
        .map(|_| {
            let (socket, pass) = (socket.clone(), pass.clone());
            tokio::spawn(async move {
                let mut client = Client::connect(&socket).await;
                client.send(1, attach_child(&pass, None)).await;
                let mut greeting = Vec::new();
                while let ServerMessage::Notification(message) = client.recv_within(PATIENCE).await
                {
                    greeting.push(message.notification);
                }
                client
                    .send(
                        2,
                        Request::SubmitInput {
                            chat: chat_of(&greeting),
                            client_ref: 1,
                            text: "child task".to_owned(),
                            skip_relation: false,
                        },
                    )
                    .await;
                client
            })
        })
        .collect();
    let running = drive(&mut family.flow.engine, async {
        let mut clients = Vec::new();
        for task in tasks {
            clients.push(task.await.unwrap());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        let running = rss_kib();
        drop(clients);
        running
    })
    .await;
    // 측정 구간은 끊기 전까지다. 끊긴 하위 접속의 정리는 완료를 기다린 뒤에 확인한다
    drive_until(&mut family.flow.engine, PATIENCE, |engine| {
        engine.passes.running_total() == 0
    })
    .await;
    let _ = stop_tx.send(());
    let samples = drive(&mut family.flow.engine, async { probe.await.unwrap() }).await;
    let mut samples = samples;
    samples.sort();
    let after = rss_kib();
    let stops: Vec<_> = groups
        .into_iter()
        .map(|group| {
            let supervisor = supervisor.clone();
            tokio::spawn(async move {
                supervisor
                    .stop_tree(group, crate::processes::StopScope::Whole)
                    .await
            })
        })
        .collect();
    for stop in stops {
        stop.await.unwrap().expect("sleep should stop");
    }
    let line = format!(
        "children={count} probes={} median={:?} p95={:?} max={:?} rss_before={before}KiB rss_running={running}KiB rss_after={after}KiB per_child={}KiB",
        samples.len(),
        percentile(&samples, 50),
        percentile(&samples, 95),
        samples.last().copied().unwrap_or_default(),
        running.saturating_sub(before) / count as u64,
    );
    writeln!(std::io::stderr(), "{line}").unwrap();
    assert_eq!(family.flow.engine.passes.running_total(), 0, "{line}");
}

#[tokio::test]
#[ignore = "부하 시험. 실행 명령은 파일 머리에 있다"]
async fn load_10_children() {
    measure(10).await;
}

#[tokio::test]
#[ignore = "부하 시험. 실행 명령은 파일 머리에 있다"]
async fn load_50_children() {
    measure(50).await;
}

#[tokio::test]
#[ignore = "부하 시험. 실행 명령은 파일 머리에 있다"]
async fn load_100_children() {
    measure(100).await;
}
