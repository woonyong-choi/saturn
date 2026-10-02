use std::collections::VecDeque;
use std::sync::Mutex as StdMutex;

use saturn_core::routers::{Answer, AnswerKind, Question, QuestionSetId};
use serde_json::json;

use super::wire::{merge_responses, split_choices, split_request};
use super::*;
use crate::secrets::{KeySource, RouterKey, SecretStore, StorageMode};

pub(crate) const KEY: &str = "sk-router-test-0123456789";

/// 주소, 헤더, 본문.
pub(crate) type Call = (String, Vec<(String, String)>, Option<String>);

/// 기록한 응답을 차례로 돌려주고 받은 요청을 남긴다.
#[derive(Debug, Default)]
pub(crate) struct FakeTransport {
    replies: StdMutex<VecDeque<Result<HttpReply, TransportError>>>,
    pub(crate) calls: StdMutex<Vec<Call>>,
    /// 다음 호출 하나가 열릴 때까지 답을 미룬다.
    gate: StdMutex<Option<Arc<tokio::sync::Notify>>>,
}

impl FakeTransport {
    pub(crate) fn new(replies: Vec<Result<HttpReply, TransportError>>) -> Arc<Self> {
        Arc::new(Self {
            replies: StdMutex::new(replies.into()),
            calls: StdMutex::default(),
            gate: StdMutex::default(),
        })
    }

    /// 다음 호출은 돌려받은 `Notify`에 `notify_one`을 부를 때까지 답하지 않는다.
    pub(crate) fn hold_next_call(&self) -> Arc<tokio::sync::Notify> {
        let gate = Arc::new(tokio::sync::Notify::new());
        *self.gate.lock().unwrap() = Some(Arc::clone(&gate));
        gate
    }

    pub(crate) fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }
}

impl Transport for FakeTransport {
    fn send<'a>(
        &'a self,
        url: &'a str,
        headers: Vec<(String, String)>,
        body: Option<String>,
        _timeout: Duration,
    ) -> TransportFuture<'a> {
        self.calls
            .lock()
            .unwrap()
            .push((url.to_owned(), headers, body));
        let reply = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Err(TransportError::BeforeSend));
        let gate = self.gate.lock().unwrap().take();
        Box::pin(async move {
            if let Some(gate) = gate {
                gate.notified().await;
            }
            reply
        })
    }
}

pub(crate) fn ok(body: &str) -> Result<HttpReply, TransportError> {
    status(200, body, Vec::new())
}

pub(crate) fn status(
    code: u16,
    body: &str,
    headers: Vec<(&str, &str)>,
) -> Result<HttpReply, TransportError> {
    Ok(HttpReply {
        status: code,
        headers: headers
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect(),
        body: body.to_owned(),
    })
}

#[test]
fn reply_debug_hides_body_and_headers() {
    let reply = status(401, KEY, vec![("authorization", KEY)]).unwrap();
    let debug = format!("{reply:?}");
    assert!(!debug.contains(KEY));
    assert!(debug.contains("body_len"));
}

pub(crate) async fn secrets_with_key(dir: &std::path::Path) -> SharedSecrets {
    let mut store = SecretStore::with_key_file(dir.join("router.key"), StorageMode::Standard);
    store
        .save(RouterKey::new(KEY.to_owned()).unwrap(), KeySource::Stored)
        .await
        .unwrap();
    Arc::new(tokio::sync::Mutex::new(store))
}

pub(crate) fn router(secrets: SharedSecrets, transport: Arc<FakeTransport>) -> RemoteRouter {
    let retry = RetryPolicy {
        response_timeout: Duration::from_secs(1),
    };
    RemoteRouter::with_transport(
        "https://api.typesafe.ai",
        "jev-1.13.0".to_owned(),
        secrets,
        retry,
        transport,
    )
}

pub(crate) fn request() -> RouterRequest {
    RouterRequest {
        model: "jev-1.13.0".to_owned(),
        state: "user: also translate the error message".to_owned(),
        sets: vec![(
            QuestionSetId {
                name: "route".to_owned(),
                major: 3,
                minor: 1,
            },
            vec![
                Question {
                    id: "keep_current".to_owned(),
                    text: "Does the input continue the running task?".to_owned(),
                    kind: AnswerKind::Noul,
                },
                Question {
                    id: "relation".to_owned(),
                    text: "How does the input relate to the running task?".to_owned(),
                    kind: AnswerKind::Choice {
                        options: vec!["refines".to_owned(), "independent".to_owned()],
                    },
                },
                Question {
                    id: "difficulty".to_owned(),
                    text: "How hard is the task?".to_owned(),
                    kind: AnswerKind::Score { levels: 3 },
                },
            ],
        )],
    }
}

pub(crate) const ANSWER: &str = r#"{"model":"jev-1.13.0","answers":{
    "keep_current":{"type":"noul","noul":0.91},
    "relation":{"type":"choice","choice":"refines","probabilities":{"refines":0.8,"independent":0.2},"confidence":0.6},
    "difficulty":{"type":"score","score":0.4,"legend":{"0":"level 1","1":"level 2","2":"level 3"},"probabilities":{"0":0.6,"1":0.4,"2":0.0},"confidence":0.3}},
    "usage":{"input_tokens":310,"output_tokens":24}}"#;

#[test]
fn endpoint_must_be_https_allowed_host() {
    for good in [
        "https://api.typesafe.ai",
        "https://api.typesafe.ai/v1/systemone",
    ] {
        assert!(validate_endpoint(good).is_ok(), "{good}");
    }
    for bad in [
        "http://api.typesafe.ai",
        "https://api.typesafe.ai.evil.com",
        "https://evil.api.typesafe.ai",
        "https://api.typesafe.ai:8443",
        "https://user@api.typesafe.ai",
        "ftp://api.typesafe.ai",
    ] {
        assert!(validate_endpoint(bad).is_err(), "{bad}");
    }
}

#[test]
fn redirect_drops_auth_headers() {
    let headers = vec![
        ("authorization".to_owned(), "Bearer x".to_owned()),
        ("X-Api-Key".to_owned(), "y".to_owned()),
        ("content-type".to_owned(), "application/json".to_owned()),
    ];

    assert_eq!(
        redirect_headers(headers),
        vec![("content-type".to_owned(), "application/json".to_owned())]
    );
}

#[tokio::test]
async fn exchange_sends_bearer_and_parses_all_answer_kinds() {
    let dir = tempfile::tempdir().unwrap();
    let transport = FakeTransport::new(vec![ok(ANSWER)]);
    let router = router(secrets_with_key(dir.path()).await, Arc::clone(&transport));

    let exchange = router.exchange(request()).await;

    let response = exchange.result.unwrap();
    assert_eq!(response.tokens, (310, 24));
    assert_eq!(
        response.answers,
        vec![
            ("keep_current".to_owned(), Answer::Noul(0.91)),
            ("relation".to_owned(), Answer::Choice(vec![0.8, 0.2])),
            ("difficulty".to_owned(), Answer::Score(vec![0.6, 0.4, 0.0])),
        ]
    );
    let calls = transport.calls();
    assert_eq!(calls[0].0, "https://api.typesafe.ai/v1/systemone");
    assert!(
        calls[0]
            .1
            .contains(&("authorization".to_owned(), format!("Bearer {KEY}")))
    );
    let body: Value = serde_json::from_str(calls[0].2.as_deref().unwrap()).unwrap();
    assert_eq!(
        body["questions"]["relation"]["criteria"],
        json!({ "refines": null, "independent": null })
    );
    assert_eq!(
        body["questions"]["difficulty"]["criteria"],
        json!(["level 1", "level 2", "level 3"])
    );
    assert!(!exchange.sent.contains(KEY));
    assert!(!format!("{router:?}").contains(KEY));
}

#[tokio::test(start_paused = true)]
async fn failures_retry_twice_five_seconds_apart_then_give_up() {
    let dir = tempfile::tempdir().unwrap();
    let secrets = secrets_with_key(dir.path()).await;
    let recovered = FakeTransport::new(vec![
        Err(TransportError::BeforeSend),
        Err(TransportError::BeforeSend),
        ok(ANSWER),
    ]);
    let gave_up = FakeTransport::new(vec![Err(TransportError::BeforeSend); 3]);

    let started = tokio::time::Instant::now();
    let first = router(Arc::clone(&secrets), Arc::clone(&recovered))
        .exchange(request())
        .await;
    let after_recovery = started.elapsed();
    let second = router(Arc::clone(&secrets), Arc::clone(&gave_up))
        .exchange(request())
        .await;
    let after_give_up = started.elapsed() - after_recovery;

    assert!(first.result.is_ok());
    assert_eq!(recovered.calls().len(), 3);
    assert_eq!(after_recovery, Duration::from_secs(10));
    assert!(matches!(second.result, Err(RouterError::NoResponse)));
    assert_eq!(gave_up.calls().len(), 3);
    assert_eq!(after_give_up, Duration::from_secs(10));
}

#[tokio::test(start_paused = true)]
async fn first_retry_waits_five_seconds_before_sending() {
    let dir = tempfile::tempdir().unwrap();
    let transport = FakeTransport::new(vec![Err(TransportError::BeforeSend), ok(ANSWER)]);
    let router = router(secrets_with_key(dir.path()).await, Arc::clone(&transport));

    let started = tokio::time::Instant::now();
    let exchange = router.exchange(request()).await;

    assert!(exchange.result.is_ok());
    assert_eq!(transport.calls().len(), 2);
    assert_eq!(started.elapsed(), Duration::from_secs(5));
}

/// 답을 정한 순서대로 주되 `None`인 호출은 끝내 응답하지 않는다. 호출 시각을 남긴다.
#[derive(Debug)]
struct ScriptedTransport {
    script: StdMutex<VecDeque<Option<Result<HttpReply, TransportError>>>>,
    started: tokio::time::Instant,
    called_at: StdMutex<Vec<Duration>>,
}

impl ScriptedTransport {
    fn new(script: Vec<Option<Result<HttpReply, TransportError>>>) -> Arc<Self> {
        Arc::new(Self {
            script: StdMutex::new(script.into()),
            started: tokio::time::Instant::now(),
            called_at: StdMutex::default(),
        })
    }

    fn called_at_seconds(&self) -> Vec<u64> {
        let times = self.called_at.lock().unwrap();
        times.iter().map(Duration::as_secs).collect()
    }
}

impl Transport for ScriptedTransport {
    fn send<'a>(
        &'a self,
        _url: &'a str,
        _headers: Vec<(String, String)>,
        _body: Option<String>,
        _timeout: Duration,
    ) -> TransportFuture<'a> {
        self.called_at.lock().unwrap().push(self.started.elapsed());
        let step = self.script.lock().unwrap().pop_front().flatten();
        Box::pin(async move {
            match step {
                Some(reply) => reply,
                None => std::future::pending().await,
            }
        })
    }
}

fn scripted_router(secrets: SharedSecrets, transport: Arc<ScriptedTransport>) -> RemoteRouter {
    RemoteRouter::with_transport(
        "https://api.typesafe.ai",
        "jev-1.13.0".to_owned(),
        secrets,
        RetryPolicy::default(),
        transport,
    )
}

#[tokio::test(start_paused = true)]
async fn no_response_at_all_gives_up_within_fifteen_seconds() {
    let dir = tempfile::tempdir().unwrap();
    let transport = ScriptedTransport::new(vec![None, None, None]);
    let router = scripted_router(secrets_with_key(dir.path()).await, Arc::clone(&transport));

    let started = tokio::time::Instant::now();
    let exchange = router.exchange(request()).await;

    assert!(matches!(
        exchange.result,
        Err(RouterError::TimedOutAfterSend)
    ));
    assert_eq!(started.elapsed(), Duration::from_secs(15));
    assert_eq!(transport.called_at_seconds(), vec![0, 10]);
    assert_eq!(exchange.unknown_cost_calls, 2);
}

#[tokio::test(start_paused = true)]
async fn immediate_failure_retries_at_five_and_ten_seconds_then_gives_up_at_the_deadline() {
    let dir = tempfile::tempdir().unwrap();
    let transport = ScriptedTransport::new(vec![
        Some(Err(TransportError::BeforeSend)),
        Some(Err(TransportError::BeforeSend)),
        None,
    ]);
    let router = scripted_router(secrets_with_key(dir.path()).await, Arc::clone(&transport));

    let started = tokio::time::Instant::now();
    let exchange = router.exchange(request()).await;

    assert!(matches!(
        exchange.result,
        Err(RouterError::TimedOutAfterSend)
    ));
    assert_eq!(started.elapsed(), Duration::from_secs(10));
    assert_eq!(transport.called_at_seconds(), vec![0, 5, 10]);
    assert_eq!(exchange.unknown_cost_calls, 1);
}

#[tokio::test(start_paused = true)]
async fn attempt_started_near_the_deadline_waits_only_the_remaining_time() {
    let dir = tempfile::tempdir().unwrap();
    let transport = ScriptedTransport::new(vec![Some(Err(TransportError::BeforeSend)), None, None]);
    let router = scripted_router(secrets_with_key(dir.path()).await, Arc::clone(&transport));

    let started = tokio::time::Instant::now();
    let exchange = router.exchange(request()).await;

    assert!(matches!(
        exchange.result,
        Err(RouterError::TimedOutAfterSend)
    ));
    assert_eq!(started.elapsed(), Duration::from_secs(10));
    assert_eq!(transport.called_at_seconds(), vec![0, 5]);
    assert_eq!(exchange.unknown_cost_calls, 1);
}

#[tokio::test(start_paused = true)]
async fn attempt_after_a_slow_first_failure_is_cut_at_the_new_deadline() {
    let dir = tempfile::tempdir().unwrap();
    let transport = ScriptedTransport::new(vec![None, Some(Err(TransportError::BeforeSend)), None]);
    let router = scripted_router(secrets_with_key(dir.path()).await, Arc::clone(&transport));

    let started = tokio::time::Instant::now();
    let exchange = router.exchange(request()).await;

    assert!(matches!(
        exchange.result,
        Err(RouterError::TimedOutAfterSend)
    ));
    assert_eq!(started.elapsed(), Duration::from_secs(15));
    assert_eq!(transport.called_at_seconds(), vec![0, 10, 15]);
    assert_eq!(exchange.unknown_cost_calls, 2);
}

#[tokio::test(start_paused = true)]
async fn timeout_after_send_is_retried_and_counted_as_unknown_cost() {
    let dir = tempfile::tempdir().unwrap();
    let secrets = secrets_with_key(dir.path()).await;
    let recovered = FakeTransport::new(vec![Err(TransportError::AfterSend), ok(ANSWER)]);
    let timed_out = FakeTransport::new(vec![Err(TransportError::AfterSend); 3]);

    let first = router(Arc::clone(&secrets), Arc::clone(&recovered))
        .exchange(request())
        .await;
    let second = router(secrets, Arc::clone(&timed_out))
        .exchange(request())
        .await;

    assert!(first.result.is_ok());
    assert_eq!(first.unknown_cost_calls, 1);
    assert_eq!(recovered.calls().len(), 2);
    assert!(matches!(second.result, Err(RouterError::TimedOutAfterSend)));
    assert_eq!(second.unknown_cost_calls, 3);
    assert_eq!(timed_out.calls().len(), 3);
    assert!(second.received.is_none());
}

#[tokio::test(start_paused = true)]
async fn rate_limit_retries_on_the_same_interval_ignoring_retry_after() {
    let dir = tempfile::tempdir().unwrap();
    let secrets = secrets_with_key(dir.path()).await;
    let limited = FakeTransport::new(vec![
        status(429, "{}", vec![("retry-after", "60")]),
        status(529, "{}", Vec::new()),
        ok(ANSWER),
    ]);
    let exhausted = FakeTransport::new(vec![status(429, "{}", Vec::new()); 3]);

    let started = tokio::time::Instant::now();
    let first = router(Arc::clone(&secrets), Arc::clone(&limited))
        .exchange(request())
        .await;
    let after_recovery = started.elapsed();
    let second = router(secrets, Arc::clone(&exhausted))
        .exchange(request())
        .await;

    assert!(first.result.is_ok());
    assert_eq!(limited.calls().len(), 3);
    assert_eq!(after_recovery, Duration::from_secs(10));
    assert!(matches!(second.result, Err(RouterError::RateLimited)));
    assert_eq!(exhausted.calls().len(), 3);
}

#[tokio::test(start_paused = true)]
async fn auth_failure_and_invalid_status_are_not_retried() {
    let dir = tempfile::tempdir().unwrap();
    let secrets = secrets_with_key(dir.path()).await;
    let rejected = FakeTransport::new(vec![status(401, "{\"error\":\"bad key\"}", Vec::new())]);
    let broken = FakeTransport::new(vec![status(500, "{}", Vec::new())]);

    let first = router(Arc::clone(&secrets), Arc::clone(&rejected))
        .exchange(request())
        .await;
    let second = router(secrets, Arc::clone(&broken))
        .exchange(request())
        .await;

    assert!(matches!(first.result, Err(RouterError::Unauthorized)));
    assert_eq!(rejected.calls().len(), 1);
    assert!(matches!(second.result, Err(RouterError::Invalid { .. })));
    assert_eq!(broken.calls().len(), 1);
}

#[tokio::test]
async fn bad_probabilities_are_invalid() {
    let dir = tempfile::tempdir().unwrap();
    let body = r#"{"model":"jev-1.13.0","answers":{"keep_current":{"noul":1.7}},"usage":{}}"#;
    let transport = FakeTransport::new(vec![ok(body)]);
    let router = router(secrets_with_key(dir.path()).await, transport);

    let exchange = router.exchange(request()).await;

    assert!(matches!(exchange.result, Err(RouterError::Invalid { .. })));
    assert_eq!(exchange.received.as_deref(), Some(body));
}

#[tokio::test]
async fn redirect_is_followed_only_to_allowed_host_without_auth() {
    let dir = tempfile::tempdir().unwrap();
    let secrets = secrets_with_key(dir.path()).await;
    let allowed = FakeTransport::new(vec![
        status(
            307,
            "",
            vec![("location", "https://api.typesafe.ai/v1/systemone2")],
        ),
        ok(ANSWER),
    ]);
    let outside = FakeTransport::new(vec![status(
        307,
        "",
        vec![("location", "https://evil.example/steal")],
    )]);

    router(Arc::clone(&secrets), Arc::clone(&allowed))
        .exchange(request())
        .await;
    let refused = router(secrets, Arc::clone(&outside))
        .exchange(request())
        .await;

    let calls = allowed.calls();
    assert_eq!(calls.len(), 2);
    assert!(calls[1].1.iter().all(|(name, _)| name != "authorization"));
    assert_eq!(outside.calls().len(), 1);
    assert!(matches!(refused.result, Err(RouterError::Invalid { .. })));
}

#[tokio::test]
async fn check_lists_models_then_routes_once() {
    let dir = tempfile::tempdir().unwrap();
    let secrets = secrets_with_key(dir.path()).await;
    let good = FakeTransport::new(vec![
        ok(r#"{"models":[{"name":"jev-latest"}]}"#),
        ok(
            r#"{"model":"jev-1.13.0","answers":{"saturn_check":{"type":"noul","noul":0.7}},"usage":{"input_tokens":5,"output_tokens":1}}"#,
        ),
    ]);
    let bad_key = FakeTransport::new(vec![status(401, "{}", Vec::new())]);

    router(Arc::clone(&secrets), Arc::clone(&good))
        .check()
        .await
        .unwrap();
    let error = router(secrets, bad_key).check().await.unwrap_err();

    let calls = good.calls();
    assert_eq!(calls[0].0, "https://api.typesafe.ai/v1/models");
    assert!(calls[0].2.is_none());
    assert!(matches!(error, RouterError::Unauthorized));
}

#[tokio::test]
async fn missing_key_is_unauthorized_without_sending() {
    let dir = tempfile::tempdir().unwrap();
    let empty = SecretStore::with_key_file(dir.path().join("none.key"), StorageMode::Standard);
    let transport = FakeTransport::new(vec![ok(ANSWER)]);
    let router = router(
        Arc::new(tokio::sync::Mutex::new(empty)),
        Arc::clone(&transport),
    );

    let exchange = router.exchange(request()).await;

    assert!(matches!(exchange.result, Err(RouterError::Unauthorized)));
    assert!(transport.calls().is_empty());
}

#[test]
fn large_requests_are_split_by_question() {
    let mut big = request();
    big.state = "s".repeat(20 * 1024);
    let long_text = "q".repeat(15 * 1024);
    for question in &mut big.sets[0].1 {
        question.text = long_text.clone();
    }

    let parts = split_request(big);
    let small = split_request(request());

    assert_eq!(parts.len(), 3);
    assert!(parts.iter().all(|part| part.state.len() == 20 * 1024));
    assert_eq!(parts[2].sets[0].1[0].id, "difficulty");
    assert_eq!(small.len(), 1);
    assert_eq!(small[0].sets[0].1.len(), 3);
}

#[test]
fn many_choices_split_and_merge_back() {
    let options: Vec<String> = (0..600).map(|n| format!("model-{n}")).collect();
    let question = Question {
        id: "target_model".to_owned(),
        text: "Which model?".to_owned(),
        kind: AnswerKind::Choice { options },
    };
    let chunks = split_choices(&question);
    assert_eq!(chunks.len(), 3);
    assert!(chunks.iter().all(|chunk| matches!(&chunk.kind, AnswerKind::Choice { options } if options.len() <= MAX_CHOICES)));
    let mut request = request();
    request.sets[0].1 = vec![question];
    let answer = |id: &str, len: usize, hot: Option<usize>| {
        let mut probabilities = vec![0.0; len];
        match hot {
            Some(index) => probabilities[index] = 1.0,
            None => probabilities[len - 1] = 1.0,
        }
        (id.to_owned(), Answer::Choice(probabilities))
    };
    let part = RouterResponse {
        model: "jev-1.13.0".to_owned(),
        answers: vec![
            answer("target_model#0", 255, None),
            answer("target_model#1", 255, Some(3)),
            answer("target_model#2", 93, None),
        ],
        tokens: (10, 1),
    };

    let merged = merge_responses(&request, vec![part]);

    let Answer::Choice(probabilities) = &merged.answers[0].1 else {
        panic!("choice answer expected");
    };
    assert_eq!(probabilities.len(), 600);
    assert!((probabilities[254 + 3] - 1.0).abs() < 1e-9);
    assert!((probabilities.iter().sum::<f64>() - 1.0).abs() < 1e-9);
}
