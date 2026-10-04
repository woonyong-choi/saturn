//! 확장 설치 결과와 목록의 문구. 판정은 engine이 보낸 값 그대로 쓰고 여기서는 글만 만든다.
//! 설계: docs/design/extensions.md#옮길-수-없는-부분-알림

use saturn_protocol::ids::Provider;
use saturn_protocol::rpc::{
    ChatNotice, ExtensionInfo, ExtensionPart, ExtensionPartKind, Injectability,
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
        ChatNotice::ExtensionFailed { name, reason } => vec![format!(
            "{prefix}{}",
            failed_line(lang, name.as_deref(), reason)
        )],
        _ => Vec::new(),
    }
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
pub(crate) fn list_lines(lang: Lang, extensions: &[ExtensionInfo]) -> Vec<String> {
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
    fn list_shows_each_part_with_a_verdict_per_provider() {
        i18n::set_provider_names([(ALPHA, "alpha"), (BETA, "beta")]);
        let kit = info(vec![part(
            ExtensionPartKind::Command,
            "review",
            true,
            false,
        )]);

        assert_eq!(
            list_lines(Lang::Ko, &[kit]),
            vec![
                "설치한 확장",
                "review-kit · /src/review-kit",
                "  명령(review) · Alpha 가능, Beta 불가"
            ]
        );
        assert_eq!(list_lines(Lang::Ko, &[]), vec!["설치한 확장 없음"]);
    }
}
