//! `/usage` 응답 행: provider·모델마다 한 행과 router마다 한 행, 각 행은 고른 범위의 합계.
//! 설계: docs/design/tui.md

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, BTreeSet, HashMap};

use saturn_protocol::event::UsageScope;
use saturn_protocol::ids::{AgentId, ChatId, Provider, RunId, SessionId, SubagentId};
use saturn_protocol::rpc::{QueryResult, UsageRange, UsageRow};

use crate::providers::Registry;
use crate::rpc::ClientId;
use crate::store::{Store, StoreError, UsageRow as StoredUsage};
use crate::{Engine, EngineError};

const ROUTER_PREFIX: &str = "router";

/// 같은 session에서 같은 에이전트와 subagent의 누적 보고 계열.
type Series = (SessionId, AgentId, Option<SubagentId>);

/// 보고 하나의 턴 값과 그 값이 걸친 실행.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TurnValue {
    tokens: [Option<u64>; 5],
    run: RunId,
    session: SessionId,
    /// 누적 보고면 같은 계열의 직전 누적 보고 실행. 그 뒤부터 `run`까지가 이 값에 합쳐졌다.
    previous_run: Option<Option<RunId>>,
}

#[derive(Debug, Default)]
struct Group {
    tokens: [Option<u64>; 5],
    runs: BTreeSet<RunId>,
}

impl Engine {
    /// `Chat` 범위는 이 클라이언트가 붙은 채팅이고, 붙은 채팅이 없으면 `folder`의 가장 최근 채팅이다.
    ///
    /// # Errors
    /// `Chat`인데 붙은 채팅도 `folder`의 채팅도 없으면 `Store(NotFound)`.
    pub(super) async fn usage_result(
        &self,
        client: ClientId,
        range: UsageRange,
        folder: Option<&str>,
    ) -> Result<QueryResult, EngineError> {
        let mut chat = self
            .attachments
            .get(&client)
            .map(|attachment| attachment.chat);
        if let (None, UsageRange::Chat, Some(folder)) = (chat, range, folder) {
            chat = self.store.latest_chat_in(folder).await?;
        }
        let rows = usage_rows(&self.store, &self.registry, range, chat).await?;
        Ok(QueryResult::Usage { range, rows })
    }
}

impl Engine {
    /// 폴더에 채팅이 없으면 `chat`이 `None`인 결과를 돌려준다.
    pub(super) async fn latest_chat_result(
        &self,
        folder: &str,
    ) -> Result<QueryResult, EngineError> {
        let chat = self.store.latest_chat_in(folder).await?;
        Ok(QueryResult::LatestChat { chat })
    }

    /// `folder`가 `None`이면 모든 폴더의 채팅.
    pub(super) async fn chat_list_result(
        &self,
        folder: Option<&str>,
    ) -> Result<QueryResult, EngineError> {
        let chats = self.store.list_chats(folder).await?;
        Ok(QueryResult::Chats { chats })
    }
}

/// 기록에 없는 비용, 맥락 정리, 채점은 `None`이다. 여러 턴을 합친 행만 `turns`를 채운다.
async fn usage_rows(
    store: &Store,
    registry: &Registry,
    range: UsageRange,
    chat: Option<ChatId>,
) -> Result<Vec<UsageRow>, StoreError> {
    let in_range = store.usage_rows(range, chat).await?;
    let turn_values = turn_values(&store.all_usage_rows().await?);
    let mut session_runs: HashMap<SessionId, Vec<RunId>> = HashMap::new();
    let mut groups: BTreeMap<String, Group> = BTreeMap::new();
    for row in &in_range {
        let Some(value) = turn_values.get(&row.id) else {
            continue;
        };
        let group = groups.entry(who(registry, row)).or_default();
        for (sum, value) in group.tokens.iter_mut().zip(value.tokens) {
            if let Some(value) = value {
                *sum = Some(sum.unwrap_or(0) + value);
            }
        }
        match value.previous_run {
            None => {
                group.runs.insert(value.run);
            }
            Some(previous) => {
                let runs = match session_runs.entry(value.session) {
                    Entry::Occupied(entry) => entry.into_mut(),
                    Entry::Vacant(entry) => entry.insert(store.session_runs(value.session).await?),
                };
                let covered = runs
                    .iter()
                    .filter(|run| previous.is_none_or(|previous| **run > previous))
                    .filter(|run| **run <= value.run);
                group.runs.extend(covered);
            }
        }
    }
    let mut rows: Vec<UsageRow> = groups
        .into_iter()
        .map(|(who, group)| {
            let turns = u32::try_from(group.runs.len()).unwrap_or(u32::MAX);
            UsageRow {
                who,
                tokens: group.tokens,
                router_calls: 0,
                estimated_cost_micros: None,
                compactions: None,
                labels: None,
                turns: (turns > 1).then_some(turns),
            }
        })
        .collect();
    rows.extend(
        store
            .router_usage(range, chat)
            .await?
            .into_iter()
            .map(|router| UsageRow {
                who: format!("{ROUTER_PREFIX} · {}", router.router),
                tokens: [router.input, None, None, router.output, None],
                router_calls: router.calls,
                estimated_cost_micros: None,
                compactions: None,
                labels: None,
                turns: None,
            }),
    );
    Ok(rows)
}

/// 요청 하나의 합계. 값이 없는 칸은 0이 아니라 비어 있다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RequestTotals {
    /// 토큰을 보고한 provider만, 이름 순서.
    pub(crate) provider_tokens: Vec<(Provider, u64)>,
    pub(crate) router_calls: u32,
    pub(crate) router_tokens: Option<u64>,
}

/// 실행 줄의 `Token`과 같은 칸: 새 입력, 캐시 쓰기, 출력, 추론. 캐시 읽기는 뺀다.
const SUMMARY_SLOTS: [usize; 4] = [0, 2, 3, 4];

/// 채팅에서 `since`(unix 밀리초) 이후에 시작한 실행의 사용량과 그 뒤의 router 호출을 합친다. 턴 값은 `/usage`와 같은 계산이라
/// 누적 보고는 직전 누적을 뺀 값만 더하고 subagent의 보고는 계열이 달라 한 번씩만 센다.
pub(crate) async fn request_totals(
    store: &Store,
    chat: ChatId,
    since: i64,
) -> Result<RequestTotals, StoreError> {
    let runs = store.runs_started_since(chat, since).await?;
    let values = turn_values(&store.all_usage_rows().await?);
    let mut per_provider: HashMap<Provider, Option<u64>> = HashMap::new();
    for row in store.usage_rows(UsageRange::Chat, Some(chat)).await? {
        let Some(value) = values
            .get(&row.id)
            .filter(|value| runs.contains(&value.run))
        else {
            continue;
        };
        let reported = SUMMARY_SLOTS
            .iter()
            .filter_map(|slot| value.tokens[*slot])
            .reduce(|a, b| a + b);
        let Some(reported) = reported else {
            continue;
        };
        let sum = per_provider.entry(row.provider).or_default();
        *sum = Some(sum.unwrap_or(0) + reported);
    }
    let mut provider_tokens: Vec<(Provider, u64)> = per_provider
        .into_iter()
        .filter_map(|(provider, tokens)| Some((provider, tokens?)))
        .collect();
    provider_tokens.sort_by_key(|(provider, _)| provider.to_string());
    let (router_calls, router_tokens) = store.router_usage_since(chat, since).await?;
    Ok(RequestTotals {
        provider_tokens,
        router_calls,
        router_tokens,
    })
}

/// 모델을 보고하지 않았으면 provider 이름만.
fn who(registry: &Registry, row: &StoredUsage) -> String {
    let provider = registry.display_name(row.provider);
    match &row.report.model {
        Some(model) => format!("{provider} · {model}"),
        None => provider.to_owned(),
    }
}

/// 누적 보고는 같은 계열의 칸마다 가장 가까운 앞 값을 빼고, 앞 값보다 작으면 그 칸을 `None`으로 둔다.
fn turn_values(all: &[StoredUsage]) -> HashMap<u64, TurnValue> {
    let mut last: HashMap<Series, ([Option<u64>; 5], RunId)> = HashMap::new();
    let mut values = HashMap::with_capacity(all.len());
    for row in all {
        let raw = tokens_of(row);
        if row.report.scope != UsageScope::ThreadCumulative {
            values.insert(row.id, turn_value(row, raw, None));
            continue;
        }
        let series = (row.session, row.report.agent, row.report.subagent.clone());
        let (seen, previous_run) = match last.get(&series) {
            Some((seen, run)) => (*seen, Some(*run)),
            None => ([None; 5], None),
        };
        let mut tokens = [None; 5];
        let mut updated = seen;
        for slot in 0..5 {
            let Some(value) = raw[slot] else { continue };
            tokens[slot] = match seen[slot] {
                Some(before) => value.checked_sub(before),
                None => Some(value),
            };
            updated[slot] = Some(value);
        }
        last.insert(series, (updated, row.run));
        values.insert(row.id, turn_value(row, tokens, Some(previous_run)));
    }
    values
}

fn turn_value(
    row: &StoredUsage,
    tokens: [Option<u64>; 5],
    previous_run: Option<Option<RunId>>,
) -> TurnValue {
    TurnValue {
        tokens,
        run: row.run,
        session: row.session,
        previous_run,
    }
}

/// 순서는 protocol `UsageRow::tokens`와 같다.
fn tokens_of(row: &StoredUsage) -> [Option<u64>; 5] {
    let report = &row.report;
    [
        report.input,
        report.cache_read,
        report.cache_write,
        report.output,
        report.reasoning,
    ]
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime};

    use saturn_core::queue::Permission;
    use saturn_core::routers::{Method, QuestionSetId};
    use saturn_core::sessions::{AgentRole, SessionRecord};
    use saturn_protocol::event::UsageReport;
    use saturn_protocol::ids::{LedgerSeq, Provider, SettingsRevision, TaskId};
    use saturn_protocol::state::{EffectScope, SessionState};

    use super::*;
    use crate::secrets::Masker;
    use crate::store::{JudgmentOutcome, NewInput, NewJudgment, NewRun};

    struct Fixture {
        _dir: tempfile::TempDir,
        store: Store,
        chat: ChatId,
    }

    impl Fixture {
        async fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let (store, _) = Store::open(dir.path()).await.unwrap();
            let chat = store.create_chat(PathBuf::from("/work")).await.unwrap();
            Self {
                _dir: dir,
                store,
                chat,
            }
        }

        async fn session(&self, id: u64, provider: Provider) -> SessionId {
            self.session_in(self.chat, id, provider).await
        }

        async fn session_in(&self, chat: ChatId, id: u64, provider: Provider) -> SessionId {
            let session = SessionId(id);
            self.store
                .upsert_session(&SessionRecord {
                    id: session,
                    chat,
                    agent: AgentId(1),
                    role: AgentRole::Main,
                    provider,
                    provider_session: None,
                    model: None,
                    state: SessionState::Open,
                    delivered: LedgerSeq(0),
                    idle_since: None,
                })
                .await
                .unwrap();
            session
        }

        async fn run(&self, session: SessionId, provider: Provider) -> RunId {
            self.run_in(self.chat, session, provider).await
        }

        async fn run_in(&self, chat: ChatId, session: SessionId, provider: Provider) -> RunId {
            let input = self
                .store
                .accept_input(&NewInput {
                    chat,
                    text: "go".to_owned(),
                    settings: SettingsRevision(1),
                    permission: Permission::Write,
                    workdir: PathBuf::from("/work"),
                    pinned_model: None,
                    skip_relation: false,
                })
                .await
                .unwrap();
            self.store
                .start_run(&NewRun {
                    input: Some(input),
                    task: TaskId(1),
                    agent: AgentId(1),
                    session,
                    provider,
                    effect_scope: EffectScope::NetworkPossible,
                })
                .await
                .unwrap()
        }

        async fn report(
            &self,
            run: RunId,
            session: SessionId,
            scope: UsageScope,
            model: &str,
            input: u64,
            output: Option<u64>,
        ) {
            let report = UsageReport {
                agent: AgentId(1),
                subagent: None,
                model: Some(model.to_owned()),
                scope,
                input: Some(input),
                cache_read: None,
                cache_write: None,
                output,
                reasoning: None,
            };
            self.store
                .record_usage(run, session, &report)
                .await
                .unwrap();
        }

        async fn judgment(&self, tokens: (u64, u64)) {
            self.judgment_with(Some(tokens)).await;
        }

        async fn judgment_with(&self, tokens: Option<(u64, u64)>) {
            let masker = Masker::default();
            self.store
                .record_judgment(&NewJudgment {
                    chat: self.chat,
                    input: None,
                    method: Method::Jev,
                    router: "jev".to_owned(),
                    model: ("jev-1.13.0".to_owned(), None),
                    question_sets: vec![QuestionSetId {
                        name: "route".to_owned(),
                        major: 1,
                        minor: 0,
                    }],
                    settings: SettingsRevision(1),
                    sent: masker.mask("{}"),
                    received: None,
                    answers: Vec::new(),
                    fallbacks: Vec::new(),
                    tokens,
                    started_at: SystemTime::now(),
                    elapsed: Duration::ZERO,
                    outcome: JudgmentOutcome::Ok,
                    router_version: "jev-1.13.0".to_owned(),
                    thresholds: Vec::new(),
                    asked_with: None,
                })
                .await
                .unwrap();
        }
    }

    #[tokio::test]
    async fn usage_rows_group_by_provider_model_and_router() {
        let fixture = Fixture::new().await;
        let codex = fixture
            .session(1, crate::providers::test_support::CODEX)
            .await;
        let first = fixture
            .run(codex, crate::providers::test_support::CODEX)
            .await;
        fixture
            .report(
                first,
                codex,
                UsageScope::ThreadCumulative,
                "gpt-5.6-terra",
                100,
                Some(10),
            )
            .await;
        let silent = fixture
            .run(codex, crate::providers::test_support::CODEX)
            .await;
        let third = fixture
            .run(codex, crate::providers::test_support::CODEX)
            .await;
        fixture
            .report(
                third,
                codex,
                UsageScope::ThreadCumulative,
                "gpt-5.6-terra",
                250,
                None,
            )
            .await;
        let claude = fixture
            .session(2, crate::providers::test_support::CLAUDE)
            .await;
        let turn = fixture
            .run(claude, crate::providers::test_support::CLAUDE)
            .await;
        fixture
            .report(turn, claude, UsageScope::MainTurn, "opus", 40, Some(4))
            .await;
        fixture.judgment((30, 3)).await;
        fixture.judgment((20, 2)).await;

        let rows = usage_rows(
            &fixture.store,
            &Registry::builtin(),
            UsageRange::Chat,
            Some(fixture.chat),
        )
        .await
        .unwrap();

        assert_eq!(silent.0 + 1, third.0);
        let who: Vec<&str> = rows.iter().map(|row| row.who.as_str()).collect();
        assert_eq!(
            who,
            vec!["claude · opus", "codex · gpt-5.6-terra", "router · jev"]
        );
        assert_eq!(rows[0].tokens, [Some(40), None, None, Some(4), None]);
        assert_eq!(rows[0].turns, None);
        assert_eq!(rows[1].tokens, [Some(250), None, None, Some(10), None]);
        assert_eq!(rows[1].turns, Some(3));
        assert_eq!(rows[1].compactions, None);
        assert_eq!(rows[1].estimated_cost_micros, None);
        assert_eq!(rows[2].tokens, [Some(50), None, None, Some(5), None]);
        assert_eq!(rows[2].router_calls, 2);
        assert_eq!(rows[2].labels, None);
    }

    #[tokio::test]
    async fn usage_rows_chat_range_without_chat_is_not_found() {
        let fixture = Fixture::new().await;

        let error = usage_rows(&fixture.store, &Registry::builtin(), UsageRange::Chat, None)
            .await
            .unwrap_err();

        assert!(matches!(error, StoreError::NotFound { .. }));
    }

    #[test]
    fn turn_values_drop_slot_when_cumulative_shrinks() {
        let row = |id: u64, run: u64, input: u64| StoredUsage {
            id,
            run: RunId(run),
            session: SessionId(1),
            provider: crate::providers::test_support::CODEX,
            report: UsageReport {
                agent: AgentId(1),
                subagent: None,
                model: None,
                scope: UsageScope::ThreadCumulative,
                input: Some(input),
                cache_read: None,
                cache_write: None,
                output: None,
                reasoning: None,
            },
            at: SystemTime::now(),
            spans_turns: false,
        };

        let values = turn_values(&[row(1, 1, 100), row(2, 2, 60)]);

        assert_eq!(values[&1].tokens[0], Some(100));
        assert_eq!(values[&2].tokens[0], None);
        assert_eq!(values[&2].previous_run, Some(Some(RunId(1))));
    }

    /// 실행과 router 호출의 시작 시각을 정해 요청 경계를 만든다.
    async fn place(fixture: &Fixture, run: RunId, at: i64) {
        sqlx::query("UPDATE runs SET started_at = ? WHERE id = ?")
            .bind(at)
            .bind(i64::try_from(run.0).unwrap())
            .execute(fixture.store.pool())
            .await
            .unwrap();
    }

    async fn place_judgments(fixture: &Fixture, at: i64) {
        sqlx::query("UPDATE judgments SET started_at = ? WHERE started_at > ?")
            .bind(at)
            .bind(at)
            .execute(fixture.store.pool())
            .await
            .unwrap();
    }

    #[tokio::test]
    #[expect(
        clippy::too_many_lines,
        reason = "요청 경계마다 값을 쌓아 가는 한 이야기라 나누면 앞 값이 흩어진다"
    )]
    async fn request_totals_take_only_their_window_and_chat_and_never_fill_missing_values() {
        let fixture = Fixture::new().await;
        let other = fixture
            .store
            .create_chat(PathBuf::from("/other"))
            .await
            .unwrap();
        let (claude, codex) = (
            crate::providers::test_support::CLAUDE,
            crate::providers::test_support::CODEX,
        );
        let claude_session = fixture.session(1, claude).await;
        let codex_session = fixture.session(2, codex).await;
        // 앞 요청의 실행과 router 호출
        let earlier = fixture.run(claude_session, claude).await;
        fixture
            .report(
                earlier,
                claude_session,
                UsageScope::MainTurn,
                "m",
                10,
                Some(5),
            )
            .await;
        place(&fixture, earlier, 1_000).await;
        fixture.judgment((1, 1)).await;
        place_judgments(&fixture, 900).await;
        // 이 요청: 누적 보고 둘은 차이만 더한다
        let first = fixture.run(codex_session, codex).await;
        fixture
            .report(
                first,
                codex_session,
                UsageScope::ThreadCumulative,
                "g",
                100,
                Some(20),
            )
            .await;
        fixture
            .report(
                first,
                codex_session,
                UsageScope::ThreadCumulative,
                "g",
                160,
                Some(40),
            )
            .await;
        place(&fixture, first, 2_000).await;
        fixture.judgment((5, 7)).await;
        place_judgments(&fixture, 2_100).await;
        // 다른 채팅의 같은 시간대 실행은 세지 않는다
        let stranger = fixture.session_in(other, 3, codex).await;
        let elsewhere = fixture.run_in(other, stranger, codex).await;
        fixture
            .report(
                elsewhere,
                stranger,
                UsageScope::MainTurn,
                "g",
                1_000,
                Some(1_000),
            )
            .await;
        place(&fixture, elsewhere, 2_500).await;

        let totals = request_totals(&fixture.store, fixture.chat, 2_000)
            .await
            .unwrap();

        assert_eq!(
            totals,
            RequestTotals {
                provider_tokens: vec![(codex, 200)],
                router_calls: 1,
                router_tokens: Some(12),
            }
        );
        // 다음 요청: 누적 값 200/60에서 앞 요청까지의 160/40을 뺀 차이만 센다. router가 토큰을 보고하지 않았다
        let second = fixture.run(codex_session, codex).await;
        fixture
            .report(
                second,
                codex_session,
                UsageScope::ThreadCumulative,
                "g",
                200,
                Some(60),
            )
            .await;
        place(&fixture, second, 3_000).await;
        fixture.judgment_with(None).await;
        place_judgments(&fixture, 3_100).await;

        let next = request_totals(&fixture.store, fixture.chat, 3_000)
            .await
            .unwrap();

        assert_eq!(
            next,
            RequestTotals {
                provider_tokens: vec![(codex, 60)],
                router_calls: 1,
                router_tokens: None,
            }
        );
        let empty = request_totals(&fixture.store, fixture.chat, 9_000)
            .await
            .unwrap();
        assert_eq!(empty.provider_tokens, Vec::new());
        assert_eq!((empty.router_calls, empty.router_tokens), (0, None));
    }
}
