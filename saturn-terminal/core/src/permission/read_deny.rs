//! `permission.read`의 `deny` 규칙을 provider의 읽기 제한이 받는 절대 경로 glob으로 바꾼다. 글자 계산만 한다.
//! 설계: docs/design/permissions.md#읽기-거부의-provider-적용

use std::path::Path;

use super::pattern::{glob_form, normalize};
use super::{PermissionTool, Rule, Verdict};

/// 번역 결과. `globs`는 `/`로 시작하고 `**`가 `/`를 포함하는 glob이다.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReadDeny {
    pub globs: Vec<String>,
    /// glob으로 옮길 수 없는 패턴 원문. 비어 있지 않으면 그 규칙은 provider에서 보장할 수 없다.
    pub unsupported: Vec<String>,
}

// cost: time O(r·p), heap O(r·p), stack O(1), alloc r
// vars: r = 규칙 수, p = 패턴 글자 수
// basis: estimate
/// 읽기 `deny` 규칙 패턴을 작업 폴더 기준 절대 경로 glob으로 바꾼다. Saturn 패턴은 `*`가 `/`도 포함하므로 `**`로 옮기고,
/// 절대 경로로 시작하지 않는 패턴은 작업 폴더 아래로 본다. `*`로 시작하는 패턴은 어느 경로에나 일치하므로 `/**/` 아래로
/// 본다. 폴더를 가리키는 패턴은 provider에서 그 아래 파일까지 막는다. `~`로 시작하거나 glob 문자(`?[]{}`)나 `\`가 든
/// 패턴은 같은 뜻으로 옮길 수 없어 `unsupported`에 담는다.
pub fn read_deny(rules: &[Rule], workdir: &Path) -> ReadDeny {
    let workdir = normalize(Path::new("/"), workdir);
    let workdir = workdir.to_string_lossy();
    let workdir = workdir.trim_end_matches('/');
    let mut plan = ReadDeny::default();
    for rule in rules
        .iter()
        .filter(|rule| rule.tool == PermissionTool::Read && rule.verdict == Verdict::Deny)
    {
        let pattern = rule.pattern.as_str();
        let Some(glob) = glob_form(pattern).filter(|_| !pattern.starts_with('~')) else {
            plan.unsupported.push(pattern.to_owned());
            continue;
        };
        let glob = if pattern.starts_with('/') {
            glob
        } else if pattern.starts_with('*') {
            let rest = glob.trim_start_matches('*');
            if rest.starts_with('/') {
                format!("/**{rest}")
            } else {
                format!("/**/*{rest}")
            }
        } else {
            format!("{workdir}/{glob}")
        };
        if !plan.globs.contains(&glob) {
            plan.globs.push(glob);
        }
    }
    plan
}
