//! 권한 규칙 판정: 모드 기본 규칙, 개별 규칙, 항상 허용으로 호출 하나의 값을 정한다. 파일과 프로세스는 다루지 않는다.
//! 설계: docs/design/permissions.md

mod pattern;

use std::path::{Path, PathBuf};

pub use pattern::{escape, is_literal, literal_prefix, may_match_prefix};
pub use saturn_protocol::event::{PermissionCall, PermissionTool};

/// 낮은 쪽부터 `ReadOnly`, `Ask`, `Edit`, `Full`. `Ask`는 묻기만 하고 거부하지 않아 `ReadOnly`보다 높다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Mode {
    ReadOnly,
    Ask,
    Edit,
    Full,
}

impl Mode {
    pub const DEFAULT: Self = Self::Edit;

    /// 모르는 이름이면 `None`.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "read-only" => Some(Self::ReadOnly),
            "ask" => Some(Self::Ask),
            "edit" => Some(Self::Edit),
            "full" => Some(Self::Full),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::Ask => "ask",
            Self::Edit => "edit",
            Self::Full => "full",
        }
    }

    /// 폴더 층 모드가 앞 층까지 합친 모드보다 낮을 때만 `Some`. 같거나 높으면 무시한다.
    pub fn lowered_by(self, folder: Self) -> Option<Self> {
        (folder < self).then_some(folder)
    }

    fn default_verdict(self, tool: PermissionTool, unit: &Unit) -> Verdict {
        match self {
            Self::ReadOnly => Verdict::Deny,
            Self::Ask => Verdict::Ask,
            Self::Edit if tool == PermissionTool::Edit && unit.is_inside => Verdict::Allow,
            Self::Edit if tool == PermissionTool::Shell && unit.is_read_only => Verdict::Allow,
            Self::Edit => Verdict::Ask,
            Self::Full => Verdict::Allow,
        }
    }
}

/// `edit` 모드가 묻지 않고 허용하는 읽기 전용 셸 명령. 인자가 붙어도 일치하고, 셸 문법이 섞이면 보지 않는다.
const READ_ONLY_SHELL: [&str; 7] = [
    "ls *",
    "cat *",
    "rg *",
    "grep *",
    "git status *",
    "git diff *",
    "git log *",
];

/// 읽기 전용 목록 명령이라도 파일을 쓰거나 외부 명령을 실행하게 만드는 긴 옵션. 하나라도 있으면 `ask`다.
/// `git`은 긴 옵션을 앞부분만 써도 받으므로 `git` 명령은 이 옵션의 앞부분도 같게 본다.
const UNSAFE_OPTIONS: [&str; 5] = [
    "--pre",
    "--hostname-bin",
    "--output",
    "--ext-diff",
    "--textconv",
];

/// 엄한 쪽이 크다: `Allow` < `Ask` < `Deny`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Verdict {
    Allow,
    Ask,
    Deny,
}

impl Verdict {
    /// 모르는 이름이면 `None`.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "allow" => Some(Self::Allow),
            "ask" => Some(Self::Ask),
            "deny" => Some(Self::Deny),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Ask => "ask",
            Self::Deny => "deny",
        }
    }
}

/// 설정 키 `permission.<도구>`가 쓰는 이름. 모르는 이름이면 `None`.
pub fn parse_tool(text: &str) -> Option<PermissionTool> {
    match text {
        "shell" => Some(PermissionTool::Shell),
        "edit" => Some(PermissionTool::Edit),
        "mcp" => Some(PermissionTool::Mcp),
        "subagent" => Some(PermissionTool::Subagent),
        _ => None,
    }
}

pub fn tool_name(tool: PermissionTool) -> &'static str {
    match tool {
        PermissionTool::Shell => "shell",
        PermissionTool::Edit => "edit",
        PermissionTool::Mcp => "mcp",
        PermissionTool::Subagent => "subagent",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub tool: PermissionTool,
    pub pattern: String,
    pub verdict: Verdict,
}

impl Rule {
    // cost: time O(p·t), heap O(t), stack O(1), alloc 4
    // vars: p = 패턴 글자 수, t = 대상 글자 수
    // basis: estimate
    fn matches(&self, tool: PermissionTool, unit: &Unit) -> bool {
        self.tool == tool
            && unit
                .candidates
                .iter()
                .any(|candidate| pattern::matches(&self.pattern, candidate))
    }

    // cost: time O(p), heap O(p), stack O(1), alloc 2
    // vars: p = 패턴 글자 수
    // basis: estimate
    /// 패턴이 `prefix`로 시작하는 대상에 일치할 수 있는지 본다.
    pub fn may_match_prefix(&self, prefix: &str) -> bool {
        pattern::may_match_prefix(&self.pattern, prefix)
    }
}

// cost: time O(r·p·t), heap O(r), stack O(1), alloc 2
// vars: r = rules.len(), p = 패턴 글자 수, t = target 글자 수
// basis: estimate
/// 규칙만으로 본 값이다. 모드와 항상 허용은 보지 않고, 일치하는 규칙이 없으면 `None`.
/// provider 설정으로 번역할 때 쓴다. 모드는 번역에 넣지 않기 때문이다.
pub fn rules_verdict(rules: &[Rule], tool: PermissionTool, target: &str) -> Option<Verdict> {
    let unit = Unit::text(target);
    let matching: Vec<&Rule> = rules
        .iter()
        .filter(|rule| rule.matches(tool, &unit))
        .collect();
    if matching.iter().any(|rule| rule.verdict == Verdict::Deny) {
        return Some(Verdict::Deny);
    }
    matching.last().map(|rule| rule.verdict)
}

/// 모드, 개별 규칙, 항상 허용. 개별 규칙은 사용자, 폴더, 채팅, 실행 층 순서로 이어 붙인 목록이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    pub mode: Mode,
    pub workdir: PathBuf,
    /// `--add-dir`와 `/add-dir`로 더한 폴더. 안의 편집은 작업 폴더 안의 편집처럼 판정한다.
    pub extra_dirs: Vec<PathBuf>,
    pub rules: Vec<Rule>,
    /// 사용자가 허가 창에서 고른 허용. `Allow` 규칙만 둔다.
    pub always: Vec<Rule>,
}

impl Policy {
    /// 개별 `Deny`가 하나라도 일치하면 순서와 상관없이 `Deny`, 아니면 마지막으로 일치한 규칙의 값이다.
    /// 일치하는 규칙이 없으면 모드 기본값이고, 항상 허용은 `Ask`일 때만 `Allow`로 바꾼다.
    /// 셸 명령은 나눈 조각마다, 편집은 경로마다 판정해 가장 엄한 값을 쓴다.
    /// 명령 치환이 든 셸 명령은 거부가 아니면 `Ask`이고, 명령 전체가 항상 허용과 일치할 때만 `Allow`다.
    pub fn decide(&self, call: &PermissionCall) -> Verdict {
        let (units, is_opaque) = self.units(call);
        let strictest = self.strictest(call.tool, &units, !is_opaque);
        if !is_opaque || strictest == Verdict::Deny {
            return strictest;
        }
        if self.always_matches(call.tool, &Unit::text(&call.target)) {
            return Verdict::Allow;
        }
        Verdict::Ask
    }

    // cost: time O(u·(r+a)·p·t), heap O(u), stack O(1), alloc u
    // vars: u = 조각이나 경로 수, r = 개별 규칙 수, a = 항상 허용 수, p = 패턴 글자 수, t = 대상 글자 수
    // basis: estimate
    /// 사용자가 `항상 허용`을 골랐을 때 저장할 허용 규칙. 규칙이 `Ask`로 판정한 조각이나 경로만 담는다.
    pub fn always_rules(&self, call: &PermissionCall) -> Vec<Rule> {
        let (units, is_opaque) = self.units(call);
        let allow = |pattern: String| Rule {
            tool: call.tool,
            pattern,
            verdict: Verdict::Allow,
        };
        if is_opaque {
            return vec![allow(escape(&call.target))];
        }
        units
            .iter()
            .filter(|unit| self.eval(call.tool, unit, false) == Verdict::Ask)
            .map(|unit| allow(unit.store.clone()))
            .collect()
    }

    // cost: time O(u·(r+a)·p·t), heap O(r), stack O(1)
    // vars: u = units.len(), r = 개별 규칙 수, a = 항상 허용 수, p = 패턴 글자 수, t = 대상 글자 수
    // basis: estimate
    fn strictest(&self, tool: PermissionTool, units: &[Unit], use_always: bool) -> Verdict {
        units
            .iter()
            .map(|unit| self.eval(tool, unit, use_always))
            .max()
            .unwrap_or(Verdict::Ask)
    }

    // cost: time O((r+a)·p·t), heap O(r), stack O(1), alloc 1
    // vars: r = 개별 규칙 수, a = 항상 허용 수, p = 패턴 글자 수, t = 대상 글자 수
    // basis: estimate
    fn eval(&self, tool: PermissionTool, unit: &Unit, use_always: bool) -> Verdict {
        let matching: Vec<&Rule> = self
            .rules
            .iter()
            .filter(|rule| rule.matches(tool, unit))
            .collect();
        if matching.iter().any(|rule| rule.verdict == Verdict::Deny) {
            return Verdict::Deny;
        }
        let verdict = matching.last().map_or_else(
            || self.mode.default_verdict(tool, unit),
            |rule| rule.verdict,
        );
        if use_always && verdict == Verdict::Ask && self.always_matches(tool, unit) {
            return Verdict::Allow;
        }
        verdict
    }

    // cost: time O(a·p·t), heap O(t), stack O(1)
    // vars: a = 항상 허용 수, p = 패턴 글자 수, t = 대상 글자 수
    // basis: estimate
    fn always_matches(&self, tool: PermissionTool, unit: &Unit) -> bool {
        self.always.iter().any(|rule| rule.matches(tool, unit))
    }

    // cost: time O(c), heap O(c), stack O(1), alloc u
    // vars: c = target 글자 수와 경로 글자 수의 합, u = 조각이나 경로 수
    // basis: estimate
    fn units(&self, call: &PermissionCall) -> (Vec<Unit>, bool) {
        match call.tool {
            PermissionTool::Shell => {
                let split = pattern::split_shell(&call.target);
                let is_single = split.is_plain && split.parts.len() == 1;
                let units = split
                    .parts
                    .iter()
                    .map(|part| Unit {
                        is_read_only: is_single && is_read_only_shell(part),
                        ..Unit::text(part)
                    })
                    .collect();
                (non_empty(units), split.is_opaque)
            }
            PermissionTool::Edit => {
                let units = call
                    .paths
                    .iter()
                    .map(|path| self.path_unit(Path::new(path)))
                    .collect();
                (non_empty(units), false)
            }
            PermissionTool::Mcp | PermissionTool::Subagent => {
                (vec![Unit::text(&call.target)], false)
            }
        }
    }

    fn path_unit(&self, path: &Path) -> Unit {
        let workdir = pattern::normalize(Path::new("/"), &self.workdir);
        let absolute = pattern::normalize(&workdir, path);
        let mut candidates = vec![absolute.to_string_lossy().into_owned()];
        let relative = absolute.strip_prefix(&workdir).ok();
        if let Some(relative) = relative {
            candidates.push(relative.to_string_lossy().into_owned());
        }
        let inside_dir = self.extra_dirs.iter().find_map(|dir| {
            absolute
                .strip_prefix(pattern::normalize(Path::new("/"), dir))
                .ok()
        });
        let inside_part = relative.or(inside_dir);
        let is_git_internal =
            inside_part.is_some_and(|part| part.components().any(|c| c.as_os_str() == ".git"));
        Unit {
            store: escape(&candidates[0]),
            is_inside: inside_part.is_some() && !is_git_internal,
            is_read_only: false,
            candidates,
        }
    }
}

/// 판정 대상 하나: 셸 명령 조각이나 편집 경로 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Unit {
    /// 패턴과 맞춰 볼 글자열. 하나라도 일치하면 일치다.
    candidates: Vec<String>,
    /// 항상 허용으로 저장할 때 쓰는 패턴.
    store: String,
    /// 작업 폴더 안의 편집인지. `.git` 아래 경로와 편집이 아니면 거짓.
    is_inside: bool,
    /// 셸 문법 없는 단일 명령이 읽기 전용 목록에 드는지. 셸 명령이 아니면 거짓.
    is_read_only: bool,
}

impl Unit {
    fn text(text: &str) -> Self {
        Self {
            candidates: vec![text.to_owned()],
            store: escape(text),
            is_inside: false,
            is_read_only: false,
        }
    }
}

fn is_read_only_shell(command: &str) -> bool {
    READ_ONLY_SHELL
        .iter()
        .any(|pattern| pattern::matches(pattern, command))
        && !has_unsafe_option(command)
}

// cost: time O(c·o), heap O(c), stack O(1), alloc 1
// vars: c = command 글자 수, o = UNSAFE_OPTIONS 수
// basis: estimate
/// 따옴표와 `\`를 걷어낸 토큰이 위험 옵션이거나 `옵션=`으로 시작하는지 본다.
fn has_unsafe_option(command: &str) -> bool {
    let plain: String = command
        .chars()
        .filter(|c| !matches!(c, '\'' | '"' | '\\'))
        .collect();
    let is_git = plain.starts_with("git ");
    plain.split_whitespace().any(|token| {
        let Some(name) = token.strip_prefix("--") else {
            return false;
        };
        let name = name.split('=').next().unwrap_or_default();
        UNSAFE_OPTIONS.iter().any(|option| {
            let option = &option[2..];
            name == option || (is_git && !name.is_empty() && option.starts_with(name))
        })
    })
}

/// 대상이 하나도 없는 호출(빈 명령, 경로를 모르는 편집)은 빈 글자열 하나로 본다.
fn non_empty(units: Vec<Unit>) -> Vec<Unit> {
    if units.is_empty() {
        vec![Unit::text("")]
    } else {
        units
    }
}

#[cfg(test)]
mod tests;
