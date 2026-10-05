//! 확장 설치 결과와 목록의 문구. 판정은 engine이 보낸 값 그대로 쓰고 여기서는 글만 만든다.
//! 설계: docs/design/extensions.md#옮길-수-없는-부분-알림

use saturn_protocol::ids::Provider;
use saturn_protocol::rpc::{
    ChatNotice, DirectInstallInfo, DirectInstallItem, DirectKind, DirectState, ExtensionInfo,
    ExtensionPart, ExtensionPartKind, Injectability,
};

use crate::i18n::{self, Lang};

fn kind_label(lang: Lang, kind: ExtensionPartKind) -> &'static str {
    lang.tr(match kind {
        ExtensionPartKind::Skill => i18n::EXT_KIND_SKILL,
        ExtensionPartKind::McpServer => i18n::EXT_KIND_MCP,
        ExtensionPartKind::Command => i18n::EXT_KIND_COMMAND,
        ExtensionPartKind::Hook => i18n::EXT_KIND_HOOK,
    })
}

fn direct_kind_label(lang: Lang, kind: DirectKind) -> &'static str {
    lang.tr(match kind {
        DirectKind::Skill => i18n::EXT_KIND_SKILL,
        DirectKind::McpServer => i18n::EXT_KIND_MCP,
        DirectKind::Command => i18n::EXT_KIND_COMMAND,
        DirectKind::Plugin => i18n::EXT_KIND_PLUGIN,
    })
}

/// 줄에 이름을 적는 항목의 최대 수(초안). 넘으면 `외 N개`로 줄인다.
const DIRECT_NAMES_SHOWN: usize = 5;

/// 한국어 주제 조사. 마지막 글자가 받침으로 끝나면 `은`, 아니면 `는`. 한글이 아니면 `는`.
fn topic_particle(word: &str) -> &'static str {
    let has_final = word.chars().last().is_some_and(|c| {
        let code = u32::from(c);
        (0xAC00..=0xD7A3).contains(&code) && (code - 0xAC00) % 28 != 0
    });
    if has_final { "은" } else { "는" }
}

fn titles(providers: &[Provider]) -> String {
    providers
        .iter()
        .map(|provider| i18n::provider_title(*provider))
        .collect::<Vec<_>>()
        .join(", ")
}

fn is_injectable(part: &ExtensionPart, provider: Provider) -> bool {
    part.verdicts
        .iter()
        .any(|(id, verdict)| *id == provider && *verdict == Injectability::Injectable)
}

/// 등록 순서대로 본 provider 목록. 판정은 부분마다 같은 provider 목록을 가진다.
fn providers_of(info: &ExtensionInfo) -> Vec<Provider> {
    info.parts
        .first()
        .map(|part| part.verdicts.iter().map(|(id, _)| *id).collect())
        .unwrap_or_default()
}

/// 확장 알림 줄. 확장 알림이 아니면 빈 목록이다.
pub(crate) fn notice_lines(lang: Lang, prefix: &str, notice: &ChatNotice) -> Vec<String> {
    match notice {
        ChatNotice::ExtensionInstalled { extension } => {
            vec![format!("{prefix}{}", installed_line(lang, extension))]
        }
        ChatNotice::ExtensionRemoved { name } => {
            vec![format!("{prefix}{}", removed_line(lang, name))]
        }
        ChatNotice::ExtensionFailed { name, reason } => {
            vec![format!(
                "{prefix}{}",
                failed_line(lang, name.as_deref(), reason)
            )]
        }
        ChatNotice::ExtensionInjectFailed {
            extension,
            part,
            provider,
            reason,
        } => vec![format!(
            "{prefix}{}",
            inject_failed_line(lang, extension, part.as_deref(), *provider, reason)
        )],
        ChatNotice::DirectInstallsFound { provider, items } => {
            direct_found_lines(lang, *provider, items)
                .into_iter()
                .map(|line| format!("{prefix}{line}"))
                .collect()
        }
        ChatNotice::ExtensionPartsNotApplied { provider, parts } => {
            not_applied_lines(lang, *provider, parts)
                .into_iter()
                .map(|line| format!("{prefix}{line}"))
                .collect()
        }
        _ => Vec::new(),
    }
}

/// provider에 직접 설치된 항목을 새로 찾았다는 줄. 옮길 수 있는 항목이 있으면 옮기는 방법을 한 줄 더한다.
pub(crate) fn direct_found_lines(
    lang: Lang,
    provider: Provider,
    items: &[DirectInstallItem],
) -> Vec<String> {
    let shown = items
        .iter()
        .take(DIRECT_NAMES_SHOWN)
        .map(|item| format!("{}({})", direct_kind_label(lang, item.kind), item.name))
        .collect::<Vec<_>>();
    let mut names = shown.join(", ");
    if items.len() > DIRECT_NAMES_SHOWN {
        names.push_str(", ");
        names.push_str(
            &lang
                .tr(i18n::EXT_DIRECT_MORE)
                .replace("{count}", &(items.len() - DIRECT_NAMES_SHOWN).to_string()),
        );
    }
    let title = i18n::provider_title(provider);
    let mut lines = vec![
        lang.tr(i18n::EXT_DIRECT_FOUND)
            .replace("{provider}", &title)
            .replace("{count}", &items.len().to_string())
            .replace("{names}", &names),
    ];
    if items.iter().any(|item| item.movable) {
        lines.push(
            lang.tr(i18n::EXT_DIRECT_ASK)
                .replace("{provider}", provider.as_str()),
        );
    }
    lines
}

/// 설치 줄. 어떤 부분도 빠지지 않으면 모든 부분을 쓸 수 있는 provider를 적고, 빠지는 부분이 있으면 그 부분만 적는다.
pub(crate) fn installed_line(lang: Lang, info: &ExtensionInfo) -> String {
    let providers = providers_of(info);
    let mut limits = Vec::new();
    for part in &info.parts {
        let usable: Vec<Provider> = providers
            .iter()
            .copied()
            .filter(|provider| is_injectable(part, *provider))
            .collect();
        let others: Vec<Provider> = providers
            .iter()
            .copied()
            .filter(|provider| !usable.contains(provider))
            .collect();
        if others.is_empty() {
            continue;
        }
        let kind = kind_label(lang, part.kind);
        let template = lang.tr(if usable.is_empty() {
            i18n::EXT_LIMIT_NONE
        } else {
            i18n::EXT_LIMIT_ONLY
        });
        limits.push(
            template
                .replace("{part}", &format!("{kind}({})", part.name))
                .replace("{topic}", topic_particle(kind))
                .replace("{only}", &titles(&usable))
                .replace("{others}", &titles(&others)),
        );
    }
    let fully: Vec<Provider> = providers
        .iter()
        .copied()
        .filter(|provider| info.parts.iter().all(|part| is_injectable(part, *provider)))
        .collect();
    let line = if !limits.is_empty() {
        lang.tr(i18n::EXT_INSTALLED_LIMITED)
            .replace("{limits}", &limits.join(" · "))
    } else if fully.is_empty() {
        lang.tr(i18n::EXT_INSTALLED_BARE).to_owned()
    } else {
        lang.tr(i18n::EXT_INSTALLED_USABLE)
            .replace("{providers}", &titles(&fully))
    };
    line.replace("{name}", &info.name)
}

/// provider가 바뀌어 적용되지 않는 부분마다 한 줄.
pub(crate) fn not_applied_lines(
    lang: Lang,
    provider: Provider,
    parts: &[(String, ExtensionPartKind, String)],
) -> Vec<String> {
    parts
        .iter()
        .map(|(extension, kind, name)| {
            let label = kind_label(lang, *kind);
            lang.tr(i18n::EXT_NOT_APPLIED)
                .replace("{extension}", extension)
                .replace("{part}", &format!("{label}({name})"))
                .replace("{topic}", topic_particle(label))
                .replace("{provider}", &i18n::provider_title(provider))
        })
        .collect()
}

/// 주입하지 못한 줄. `part`가 없으면 확장 전체다.
pub(crate) fn inject_failed_line(
    lang: Lang,
    extension: &str,
    part: Option<&str>,
    provider: Provider,
    reason: &str,
) -> String {
    let target = match part {
        Some(part) => format!("{extension}({part})"),
        None => extension.to_owned(),
    };
    format!(
        "{} · {} · {target} · {reason}",
        lang.tr(i18n::EXT_INJECT_FAILED),
        i18n::provider_title(provider)
    )
}

pub(crate) fn removed_line(lang: Lang, name: &str) -> String {
    lang.tr(i18n::EXT_REMOVED).replace("{name}", name)
}

pub(crate) fn failed_line(lang: Lang, name: Option<&str>, reason: &str) -> String {
    match name {
        Some(name) => format!("{} · {name} · {reason}", lang.tr(i18n::EXT_FAILED)),
        None => format!("{} · {reason}", lang.tr(i18n::EXT_FAILED)),
    }
}

fn verdict_label(lang: Lang, verdict: Injectability) -> &'static str {
    lang.tr(match verdict {
        Injectability::Injectable => i18n::EXT_USABLE,
        Injectability::Unavailable => i18n::EXT_UNUSABLE,
        Injectability::Unknown => i18n::EXT_UNKNOWN,
    })
}

/// `/extensions` 목록. 확장마다 한 줄과 부분마다 들여 쓴 한 줄.
pub(crate) fn list_lines(
    lang: Lang,
    extensions: &[ExtensionInfo],
    direct: &[DirectInstallInfo],
) -> Vec<String> {
    let mut lines = installed_list_lines(lang, extensions);
    if !direct.is_empty() {
        lines.push(lang.tr(i18n::EXT_DIRECT_HEAD).to_owned());
        for info in direct {
            let note = match (info.state, info.movable) {
                (Some(DirectState::Moved), _) => i18n::EXT_DIRECT_MOVED,
                (_, true) => i18n::EXT_DIRECT_MOVABLE,
                (_, false) => i18n::EXT_DIRECT_ONLY,
            };
            lines.push(format!(
                "  {} · {}({}) · {}",
                i18n::provider_title(info.provider),
                direct_kind_label(lang, info.kind),
                info.name,
                lang.tr(note)
            ));
        }
    }
    lines
}

fn installed_list_lines(lang: Lang, extensions: &[ExtensionInfo]) -> Vec<String> {
    if extensions.is_empty() {
        return vec![lang.tr(i18n::EXT_LIST_EMPTY).to_owned()];
    }
    let mut lines = vec![lang.tr(i18n::EXT_LIST_HEAD).to_owned()];
    for info in extensions {
        lines.push(format!("{} · {}", info.name, info.source));
        for part in &info.parts {
            let verdicts = part
                .verdicts
                .iter()
                .map(|(provider, verdict)| {
                    format!(
                        "{} {}",
                        i18n::provider_title(*provider),
                        verdict_label(lang, *verdict)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(format!(
                "  {}({}) · {verdicts}",
                kind_label(lang, part.kind),
                part.name
            ));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use saturn_protocol::rpc::ExtensionPart;

    use super::*;

    const ALPHA: Provider = Provider::from_static("ext-alpha");
    const BETA: Provider = Provider::from_static("ext-beta");

    fn part(kind: ExtensionPartKind, name: &str, alpha: bool, beta: bool) -> ExtensionPart {
        let verdict = |usable| {
            if usable {
                Injectability::Injectable
            } else {
                Injectability::Unavailable
            }
        };
        ExtensionPart {
            kind,
            name: name.to_owned(),
            verdicts: vec![(ALPHA, verdict(alpha)), (BETA, verdict(beta))],
        }
    }

    fn info(parts: Vec<ExtensionPart>) -> ExtensionInfo {
        ExtensionInfo {
            name: "review-kit".to_owned(),
            source: "/src/review-kit".to_owned(),
            installed_at_ms: 0,
            parts,
        }
    }

    #[test]
    fn install_line_lists_the_providers_that_can_use_every_part() {
        i18n::set_provider_names([(ALPHA, "alpha"), (BETA, "beta")]);
        let kit = info(vec![
            part(ExtensionPartKind::Skill, "commit-helper", true, true),
            part(ExtensionPartKind::McpServer, "lint", true, true),
        ]);

        assert_eq!(
            installed_line(Lang::Ko, &kit),
            "review-kit 설치 · Alpha, Beta에서 사용 가능"
        );
    }

    #[test]
    fn install_line_names_only_the_parts_that_cannot_move() {
        i18n::set_provider_names([(ALPHA, "alpha"), (BETA, "beta")]);
        let kit = info(vec![
            part(ExtensionPartKind::Skill, "commit-helper", true, true),
            part(ExtensionPartKind::Hook, "PreToolUse", true, false),
            part(ExtensionPartKind::McpServer, "lint", false, false),
        ]);

        assert_eq!(
            installed_line(Lang::Ko, &kit),
            "review-kit 설치 · 훅(PreToolUse)은 Alpha 전용이라 Beta에서 쓰지 못함 · \
             MCP 서버(lint)는 어느 provider에서도 쓰지 못함"
        );
    }

    #[test]
    fn switch_lines_name_each_part_the_new_provider_does_not_take() {
        i18n::set_provider_names([(BETA, "beta")]);
        let parts = vec![
            (
                "review-kit".to_owned(),
                ExtensionPartKind::Hook,
                "PreToolUse".to_owned(),
            ),
            (
                "review-kit".to_owned(),
                ExtensionPartKind::McpServer,
                "lint".to_owned(),
            ),
        ];

        assert_eq!(
            not_applied_lines(Lang::Ko, BETA, &parts),
            vec![
                "review-kit의 훅(PreToolUse)은 Beta에 적용되지 않음",
                "review-kit의 MCP 서버(lint)는 Beta에 적용되지 않음"
            ]
        );
    }

    #[test]
    fn inject_failure_line_names_the_provider_and_the_part() {
        i18n::set_provider_names([(ALPHA, "alpha")]);

        assert_eq!(
            inject_failed_line(
                Lang::Ko,
                "review-kit",
                Some("lint"),
                ALPHA,
                "bad definition"
            ),
            "확장 주입 실패 · Alpha · review-kit(lint) · bad definition"
        );
        assert_eq!(
            inject_failed_line(Lang::Ko, "review-kit", None, ALPHA, "missing"),
            "확장 주입 실패 · Alpha · review-kit · missing"
        );
    }

    #[test]
    fn list_shows_each_part_with_a_verdict_per_provider() {
        i18n::set_provider_names([(ALPHA, "alpha"), (BETA, "beta")]);
        let kit = info(vec![part(
            ExtensionPartKind::Command,
            "review",
            true,
            false,
        )]);

        assert_eq!(
            list_lines(Lang::Ko, &[kit], &[]),
            vec![
                "설치한 확장",
                "review-kit · /src/review-kit",
                "  명령(review) · Alpha 가능, Beta 불가"
            ]
        );
        assert_eq!(list_lines(Lang::Ko, &[], &[]), vec!["설치한 확장 없음"]);
    }

    #[test]
    fn direct_items_are_listed_with_their_state_and_asked_about_with_the_move_command() {
        i18n::set_provider_names([(ALPHA, "alpha"), (BETA, "beta")]);
        let direct = |kind, name: &str, movable, state| DirectInstallInfo {
            provider: ALPHA,
            kind,
            name: name.to_owned(),
            movable,
            state,
        };
        let listed = list_lines(
            Lang::Ko,
            &[],
            &[
                direct(
                    DirectKind::Skill,
                    "commit-helper",
                    true,
                    Some(DirectState::Asked),
                ),
                direct(
                    DirectKind::McpServer,
                    "lint",
                    true,
                    Some(DirectState::Moved),
                ),
                direct(DirectKind::Plugin, "kit@market", false, None),
            ],
        );
        assert_eq!(
            listed,
            vec![
                "설치한 확장 없음",
                "provider에 직접 설치됨",
                "  Alpha · 스킬(commit-helper) · 옮길 수 있음",
                "  Alpha · MCP 서버(lint) · 옮김",
                "  Alpha · 플러그인(kit@market) · 추적만",
            ]
        );

        let item = |kind, name: &str, movable| DirectInstallItem {
            kind,
            name: name.to_owned(),
            movable,
        };
        assert_eq!(
            direct_found_lines(
                Lang::Ko,
                ALPHA,
                &[
                    item(DirectKind::Skill, "a", true),
                    item(DirectKind::Plugin, "b", false)
                ]
            ),
            vec![
                "Alpha에 직접 설치된 항목 2개 추적 · 스킬(a), 플러그인(b)",
                "Saturn에 옮겨 다른 provider에서도 쓰려면 /extensions move ext-alpha <이름>",
            ]
        );
        assert_eq!(
            direct_found_lines(Lang::Ko, ALPHA, &[item(DirectKind::Plugin, "b", false)]).len(),
            1
        );
    }
}
