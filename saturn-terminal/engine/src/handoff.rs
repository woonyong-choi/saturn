//! 기록 목록에서 새 session에 넘기는 패킷과, 돌아온 session에 붙이는 변경분을 만든다.
//! 설계: docs/design/context-management.md#패킷-구성, docs/design/providers-and-sessions.md

use std::collections::HashMap;

use saturn_core::sessions::context::ContextBudget;
use saturn_core::sessions::memo::{ToolKind, tool_memo};
use saturn_core::sessions::packet::{
    CompetingItem, Entry, PacketOutcome, PacketSource, RECENT_TURNS, RecentTurn, build_packet,
};
use saturn_core::sessions::ranking::{Candidate, DEFAULT_RRF_K, rank_candidates};
use saturn_core::sessions::stamp::Stamp;
use saturn_protocol::event::{Activity, ProviderEvent, ToolCategory, ToolDetail};
use saturn_protocol::ids::{LedgerSeq, RunId, SessionId};

use crate::store::LedgerRow;

/// 새 session의 첫 턴으로 보내는 글.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Handoff {
    pub(crate) text: String,
    /// 추정 토큰 수.
    pub(crate) tokens: u64,
    /// 고정 구역이 `P_max`를 넘어 `P_hard`까지 허용했다.
    pub(crate) is_over_limit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HandoffOutcome {
    Ready(Handoff),
    /// 넘길 기록이 없다.
    Empty,
    /// 고정 구역이 `P_hard`도 넘어 새 session으로 옮기지 않는다.
    Deferred {
        constraints: Vec<String>,
    },
}

/// 한 도구 호출과 그 결과. 결과가 아직 없으면 끝나지 않은 항목이다.
struct Tool {
    seq: LedgerSeq,
    stamp: Stamp,
    title: String,
    kind: ToolKind,
    path: Option<String>,
    files: Vec<String>,
    output: Option<String>,
}

// cost: time O(L + c log c), heap O(L), stack O(1)
// vars: L = 기록 글자 수, c = 도구 호출 수
// basis: estimate
/// 순서는 후보 순위(RRF)만 쓴다. 제약 식별과 `compact` 판단 순서는 아직 넣지 않는다.
/// TODO(#90): 고정 구역의 "현재 목표"와 "끝나지 않은 항목"을 뽑는 규칙, 제약 식별(`constraints`)과 router `compact` 순서가 정해지기 전까지 목표는 마지막 입력, 제약은 빈 목록이다
pub(crate) fn build_handoff(rows: &[LedgerRow], budget: &ContextBudget) -> HandoffOutcome {
    let Some(last) = rows.last() else {
        return HandoffOutcome::Empty;
    };
    let turns = recent_turns(rows);
    let tools = tools(rows);
    let source = PacketSource {
        constraints: Vec::new(),
        goal_and_last_input: turns
            .last()
            .map(|turn| {
                vec![Entry {
                    seq: turn.seq,
                    text: turn.input.clone(),
                }]
            })
            .unwrap_or_default(),
        open_items: open_items(&tools),
        competitors: ordered_competitors(&tools, &turns),
        recent_turns: turns,
        up_to: last.seq,
    };
    match build_packet(&source, budget) {
        PacketOutcome::Ready(packet) => HandoffOutcome::Ready(Handoff {
            text: packet.text,
            tokens: packet.tokens,
            is_over_limit: packet.is_over_limit,
        }),
        PacketOutcome::Deferred { constraints } => HandoffOutcome::Deferred { constraints },
    }
}

/// 입력이 있는 실행마다 턴 하나. 기록 번호는 그 실행의 첫 이벤트 번호이고 답은 메인 에이전트 글을 이은 것이다.
fn recent_turns(rows: &[LedgerRow]) -> Vec<RecentTurn> {
    let mut order: Vec<RunId> = Vec::new();
    let mut turns: HashMap<RunId, RecentTurn> = HashMap::new();
    for row in rows {
        let Some(input) = &row.input else {
            continue;
        };
        let turn = turns.entry(row.run).or_insert_with(|| {
            order.push(row.run);
            RecentTurn {
                seq: row.seq,
                stamp: stamp_of(row),
                input: input.clone(),
                answer: String::new(),
            }
        });
        if let ProviderEvent::Text {
            subagent: None,
            text,
            ..
        } = &row.event
        {
            turn.answer.push_str(text);
        }
    }
    order
        .into_iter()
        .filter_map(|run| turns.remove(&run))
        .collect()
}

fn tools(rows: &[LedgerRow]) -> Vec<Tool> {
    let results: HashMap<&str, &str> = rows
        .iter()
        .filter_map(|row| match &row.event {
            ProviderEvent::ToolResult {
                call_id, output, ..
            } => Some((call_id.as_str(), output.as_str())),
            _ => None,
        })
        .collect();
    let exit_codes: HashMap<&str, Option<i32>> = rows
        .iter()
        .filter_map(|row| match &row.event {
            ProviderEvent::ToolResult {
                call_id, exit_code, ..
            } => Some((call_id.as_str(), *exit_code)),
            _ => None,
        })
        .collect();
    rows.iter()
        .filter_map(|row| match &row.event {
            ProviderEvent::ToolCall {
                call_id,
                activity,
                detail,
                ..
            } if detail.category.is_candidate() => Some(tool_of(
                row,
                activity,
                detail,
                results.get(call_id.as_str()).copied(),
                exit_codes.get(call_id.as_str()).copied().flatten(),
            )),
            _ => None,
        })
        .collect()
}

fn tool_of(
    row: &LedgerRow,
    activity: &Activity,
    detail: &ToolDetail,
    output: Option<&str>,
    exit_code: Option<i32>,
) -> Tool {
    let path = detail.paths.first().cloned();
    let (title, command) = match activity {
        Activity::RunningCommand { command } => (format!("Run {command}"), Some(command.clone())),
        _ if detail.paths.is_empty() => (format!("{:?}", detail.category), None),
        _ => (
            format!("{:?} {}", detail.category, detail.paths.join(" ")),
            None,
        ),
    };
    Tool {
        seq: row.seq,
        stamp: stamp_of(row),
        title,
        kind: kind_of(detail, path.as_ref(), command, exit_code),
        path,
        files: detail.paths.clone(),
        output: output.map(str::to_owned),
    }
}

/// 값을 얻은 도구 종류만 메모 틀을 쓰고 나머지는 `Other`다. 테스트 통과 수는 구조로 얻지 못해 `TestRun`을 쓰지 않는다.
fn kind_of(
    detail: &ToolDetail,
    path: Option<&String>,
    command: Option<String>,
    exit_code: Option<i32>,
) -> ToolKind {
    match (detail.category, path, command) {
        (ToolCategory::FileRead, Some(path), _) => ToolKind::FileRead {
            path: path.clone(),
            lines: detail.read_lines.map(|range| (range.first, range.last)),
        },
        (ToolCategory::FileEdit, Some(path), _) => match detail.changed {
            Some(change) => ToolKind::FileEdit {
                path: path.clone(),
                added: change.added,
                removed: change.removed,
            },
            None => ToolKind::Other,
        },
        (ToolCategory::Shell, _, Some(command)) => ToolKind::Shell { command, exit_code },
        _ => ToolKind::Other,
    }
}

fn open_items(tools: &[Tool]) -> Vec<Entry> {
    tools
        .iter()
        .filter(|tool| tool.output.is_none())
        .map(|tool| Entry {
            seq: tool.seq,
            text: format!("Unfinished tool call: {}", tool.title),
        })
        .collect()
}

fn ordered_competitors(tools: &[Tool], turns: &[RecentTurn]) -> Vec<CompetingItem> {
    let last_input = turns.last().map_or("", |turn| turn.input.as_str());
    let first_recent = turns
        .len()
        .checked_sub(RECENT_TURNS)
        .map_or(0, |skip| turns[skip].seq.0);
    let base_files: Vec<String> = tools
        .iter()
        .filter(|tool| tool.seq.0 >= first_recent)
        .flat_map(|tool| tool.files.clone())
        .collect();
    let candidates: Vec<Candidate> = tools
        .iter()
        .map(|tool| Candidate {
            seq: tool.seq,
            text: item_text(tool),
            files: tool.files.clone(),
        })
        .collect();
    rank_candidates(&candidates, &base_files, last_input, DEFAULT_RRF_K)
        .into_iter()
        .filter_map(|seq| tools.iter().find(|tool| tool.seq == seq))
        .map(|tool| CompetingItem {
            seq: tool.seq,
            stamp: tool.stamp,
            text: item_text(tool),
            memo: tool_memo(&tool.kind, tool.output.as_deref().unwrap_or_default()),
            path: tool.path.clone(),
        })
        .collect()
}

fn item_text(tool: &Tool) -> String {
    format!(
        "{}\n{}",
        tool.title,
        tool.output.as_deref().unwrap_or_default()
    )
}

fn stamp_of(row: &LedgerRow) -> Stamp {
    Stamp {
        session: row.session,
        at_ms: Some(row.at_ms),
    }
}

/// 기록 번호 뒤의 변경분 중 그 session이 낸 것은 이미 받았으므로 뺀다.
pub(crate) fn others_only(rows: Vec<LedgerRow>, session: SessionId) -> Vec<LedgerRow> {
    rows.into_iter()
        .filter(|row| row.session != session)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use saturn_protocol::event::LineRange;
    use saturn_protocol::ids::AgentId;

    use super::*;

    fn text_event(agent: AgentId, text: &str) -> ProviderEvent {
        ProviderEvent::Text {
            agent,
            subagent: None,
            text: text.to_owned(),
        }
    }

    fn budget() -> ContextBudget {
        ContextBudget {
            t_abs: 10_000,
            safety_percent: 100,
            window: 10_000,
            cache_read: 0.1,
            cache_write: 1.25,
            cache_ttl: Duration::from_secs(300),
        }
    }

    fn row(
        seq: u64,
        run: u64,
        session: u64,
        input: Option<&str>,
        event: ProviderEvent,
    ) -> LedgerRow {
        LedgerRow {
            seq: LedgerSeq(seq),
            run: RunId(run),
            session: SessionId(session),
            input: input.map(str::to_owned),
            at_ms: 1_700_000_000_000,
            event,
        }
    }

    fn read_call(call: &str, path: &str) -> ProviderEvent {
        ProviderEvent::ToolCall {
            agent: AgentId(1),
            subagent: None,
            call_id: call.to_owned(),
            activity: Activity::ReadingFile,
            detail: ToolDetail {
                category: ToolCategory::FileRead,
                paths: vec![path.to_owned()],
                read_lines: Some(LineRange { first: 1, last: 9 }),
                changed: None,
            },
        }
    }

    fn result(call: &str, output: &str) -> ProviderEvent {
        ProviderEvent::ToolResult {
            agent: AgentId(1),
            subagent: None,
            call_id: call.to_owned(),
            output: output.to_owned(),
            exit_code: None,
        }
    }

    #[test]
    fn empty_rows_have_nothing_to_hand_over() {
        assert_eq!(build_handoff(&[], &budget()), HandoffOutcome::Empty);
    }

    #[test]
    fn packet_carries_input_answer_and_tool_result_with_session_title() {
        let rows = vec![
            row(
                1,
                1,
                5,
                Some("fix the cache"),
                text_event(AgentId(1), "looking"),
            ),
            row(
                2,
                1,
                5,
                Some("fix the cache"),
                read_call("c1", "src/cache.rs"),
            ),
            row(3, 1, 5, Some("fix the cache"), result("c1", "cache body")),
            row(
                4,
                1,
                5,
                Some("fix the cache"),
                text_event(AgentId(1), " done"),
            ),
        ];

        let HandoffOutcome::Ready(handoff) = build_handoff(&rows, &budget()) else {
            panic!("packet should be ready");
        };

        assert!(handoff.text.contains("fix the cache"));
        assert!(handoff.text.contains("looking done"));
        assert!(handoff.text.contains("cache body"));
        assert!(handoff.text.contains("### Session 5"));
        assert!(!handoff.is_over_limit);
    }

    #[test]
    fn tool_call_without_a_result_is_listed_as_an_open_item() {
        let rows = vec![
            row(1, 1, 5, Some("run it"), read_call("c1", "a.rs")),
            row(2, 1, 5, Some("run it"), text_event(AgentId(1), "waiting")),
        ];

        let HandoffOutcome::Ready(handoff) = build_handoff(&rows, &budget()) else {
            panic!("packet should be ready");
        };

        assert!(handoff.text.contains("Unfinished tool call"));
    }

    #[test]
    fn others_only_drops_the_sessions_own_rows() {
        let rows = vec![
            row(1, 1, 1, Some("a"), text_event(AgentId(1), "x")),
            row(2, 2, 2, Some("b"), text_event(AgentId(1), "y")),
        ];

        let kept = others_only(rows, SessionId(1));

        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].session, SessionId(2));
    }
}
