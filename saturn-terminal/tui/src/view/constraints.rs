//! `/constraints`가 보이는 줄. 후보와 유효 제약은 색이 아니라 `[후보]`, `[유효]` 글자로 구별해 전체 화면과 plain이 같다.
//! 설계: docs/design/tui.md, docs/design/constraints.md#되돌리기

use saturn_protocol::rpc::{
    ConstraintActor, ConstraintChangeInfo, ConstraintChangeKind, ConstraintExceptionKind,
    ConstraintInfo, ConstraintStatus,
};

use crate::constraints::{DeskError, ListView, Listing};
use crate::i18n::{self, Lang};

/// 규칙은 사용자 원문이라 줄바꿈과 겹친 공백만 한 칸으로 합치고 길면 줄인다.
const RULE_CHARS: usize = 80;

fn one_line(text: &str) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.chars().count() <= RULE_CHARS {
        return joined;
    }
    let cut: String = joined.chars().take(RULE_CHARS - 1).collect();
    format!("{cut}…")
}

fn tag(lang: Lang, status: ConstraintStatus) -> &'static str {
    lang.tr(match status {
        ConstraintStatus::Candidate => i18n::CONSTRAINT_TAG_CANDIDATE,
        ConstraintStatus::Active => i18n::CONSTRAINT_TAG_ACTIVE,
        ConstraintStatus::Released => i18n::CONSTRAINT_TAG_RELEASED,
    })
}

fn constraint_line(lang: Lang, constraint: &ConstraintInfo) -> String {
    let scope = if constraint.scope.is_empty() {
        lang.tr(i18n::CONSTRAINT_SCOPE_ALL).to_owned()
    } else {
        constraint.scope.join(", ")
    };
    let mut line = format!(
        "[{}] #{} · {} {} · {}",
        tag(lang, constraint.status),
        constraint.id.0,
        lang.tr(i18n::CONSTRAINT_SCOPE),
        scope,
        one_line(&constraint.rule)
    );
    match &constraint.exception {
        Some((ConstraintExceptionKind::Once, _)) => {
            line.push_str(&format!(
                " · {} {}",
                lang.tr(i18n::CONSTRAINT_EXCEPTION),
                lang.tr(i18n::CONSTRAINT_PAUSED_FOR_TASK)
            ));
        }
        Some((ConstraintExceptionKind::Scoped, condition)) => {
            line.push_str(&format!(
                " · {} {}",
                lang.tr(i18n::CONSTRAINT_EXCEPTION),
                condition.as_deref().map(one_line).unwrap_or_default()
            ));
        }
        None => {}
    }
    line
}

fn change_kind(lang: Lang, kind: ConstraintChangeKind) -> &'static str {
    lang.tr(match kind {
        ConstraintChangeKind::Added => i18n::CONSTRAINT_CHANGE_ADDED,
        ConstraintChangeKind::Released => i18n::CONSTRAINT_CHANGE_RELEASED,
        ConstraintChangeKind::Excepted => i18n::CONSTRAINT_CHANGE_EXCEPTED,
        ConstraintChangeKind::Resumed => i18n::CONSTRAINT_CHANGE_RESUMED,
        ConstraintChangeKind::Restored => i18n::CONSTRAINT_CHANGE_RESTORED,
    })
}

fn actor(lang: Lang, actor: ConstraintActor) -> &'static str {
    lang.tr(match actor {
        ConstraintActor::Router => i18n::CONSTRAINT_ACTOR_ROUTER,
        ConstraintActor::User => i18n::CONSTRAINT_ACTOR_USER,
        ConstraintActor::Engine => i18n::CONSTRAINT_ACTOR_ENGINE,
    })
}

fn change_line(lang: Lang, change: &ConstraintChangeInfo) -> String {
    let mut line = format!(
        "#{} · #{} · {} · {} · {}",
        change.event,
        change.constraint.0,
        change_kind(lang, change.kind),
        actor(lang, change.actor),
        one_line(&change.rule)
    );
    if change.undoable {
        line.push_str(&format!(" · {}", lang.tr(i18n::CONSTRAINT_UNDOABLE)));
    }
    line
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 제약 수와 변경 수
// basis: estimate
/// 목록 줄. 후보는 등록 확인 중이고 패킷에는 유효 제약처럼 들어간다는 안내를 붙인다. 해제된 제약은 목록에서 빼고
/// 개수만 센다. 변경 내역에서 되돌린다.
pub(crate) fn list_lines(lang: Lang, view: ListView, listing: &Listing) -> Vec<String> {
    let count = |status| {
        listing
            .constraints
            .iter()
            .filter(|constraint| constraint.status == status)
            .count()
    };
    let mut lines = vec![
        lang.tr(i18n::CONSTRAINTS_HEADER)
            .replace("{active}", &count(ConstraintStatus::Active).to_string())
            .replace(
                "{candidate}",
                &count(ConstraintStatus::Candidate).to_string(),
            )
            .replace("{released}", &count(ConstraintStatus::Released).to_string())
            .replace("{revision}", &listing.revision.to_string()),
    ];
    match view {
        ListView::Constraints => {
            let live: Vec<&ConstraintInfo> = listing
                .constraints
                .iter()
                .filter(|constraint| constraint.status != ConstraintStatus::Released)
                .collect();
            if live.is_empty() {
                lines.push(lang.tr(i18n::CONSTRAINTS_EMPTY).to_owned());
            }
            lines.extend(
                live.iter()
                    .map(|constraint| constraint_line(lang, constraint)),
            );
            if count(ConstraintStatus::Candidate) > 0 {
                lines.push(lang.tr(i18n::CONSTRAINT_CANDIDATE_NOTE).to_owned());
            }
            lines.push(lang.tr(i18n::CONSTRAINTS_HINT).to_owned());
        }
        ListView::History => {
            if listing.changes.is_empty() {
                lines.push(lang.tr(i18n::CONSTRAINT_HISTORY_EMPTY).to_owned());
            }
            lines.extend(
                listing
                    .changes
                    .iter()
                    .map(|change| change_line(lang, change)),
            );
            lines.push(lang.tr(i18n::CONSTRAINT_HISTORY_HINT).to_owned());
        }
    }
    lines
}

/// 요청을 만들지 못한 까닭 한 줄.
pub(crate) fn error_line(lang: Lang, error: DeskError) -> String {
    let (template, id) = match error {
        DeskError::ListFirst => return lang.tr(i18n::CONSTRAINTS_LIST_FIRST).to_owned(),
        DeskError::Unknown(id) => (i18n::CONSTRAINTS_UNKNOWN, id),
        DeskError::NotActive(id) => (i18n::CONSTRAINTS_NOT_ACTIVE, id),
        DeskError::NotUndoable(id) => (i18n::CONSTRAINTS_NOT_UNDOABLE, id),
    };
    lang.tr(template).replace("{id}", &id.to_string())
}
