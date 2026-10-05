//! 확장 주입의 공통 부분: 어댑터가 설명자에 적는 주입 형식, 주입할 부분과 실패 값, 파일 배치와 정의 읽기 도우미.
//! 어댑터는 이 도우미로 배치한 뒤 provider 형식으로 바꾸는 일(설정 파일 쓰기, 실행 인자)만 한다.
//! 설계: docs/design/extensions.md#주입

use std::path::{Path, PathBuf};

use saturn_protocol::rpc::{DirectKind, ExtensionPartKind};
use serde_json::Value;

use crate::extensions::source::copy_into;

/// 어댑터가 확장 부분을 받는 provider 형식. 설명자에 적는다. 비어 있는 칸은 그 종류를 주입하지 않는다는 뜻이고,
/// 부분 종류마다 `주입 가능` 판정의 기본 답이 된다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ExtensionLayout {
    /// 주입 폴더 안에서 스킬 폴더를 두는 폴더 이름. 예: `skills`.
    pub(crate) skills_dir: Option<&'static str>,
    /// 명령 파일을 두는 폴더 이름과 파일 확장자. 예: (`commands`, `md`).
    pub(crate) commands: Option<(&'static str, &'static str)>,
    /// MCP 서버 정의를 받는다. 변환은 어댑터가 한다.
    pub(crate) mcp_servers: bool,
    /// 훅 정의를 받는다. 변환은 어댑터가 한다.
    pub(crate) hooks: bool,
}

impl ExtensionLayout {
    /// 확장을 주입하지 않는 어댑터.
    pub(crate) const NONE: Self = Self {
        skills_dir: None,
        commands: None,
        mcp_servers: false,
        hooks: false,
    };

    /// 이 종류의 부분을 받는지.
    pub(crate) fn accepts(&self, kind: ExtensionPartKind) -> bool {
        match kind {
            ExtensionPartKind::Skill => self.skills_dir.is_some(),
            ExtensionPartKind::Command => self.commands.is_some(),
            ExtensionPartKind::McpServer => self.mcp_servers,
            ExtensionPartKind::Hook => self.hooks,
        }
    }
}

/// 어댑터가 session을 열 때 주입할 확장 부분 하나. 원본은 확장 저장소에 있고 어댑터는 읽기만 한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InjectedPart {
    /// 부분이 든 확장 이름.
    pub extension: String,
    pub kind: ExtensionPartKind,
    /// 스킬은 폴더 이름, 명령은 파일 이름, MCP 서버와 훅은 정의 파일 안의 키.
    pub name: String,
    /// 스킬은 폴더, 그 밖에는 파일의 절대 경로. MCP 서버와 훅은 키가 든 JSON 파일이다.
    pub source: PathBuf,
}

/// 주입하지 못한 부분 하나. engine이 대화 기록에 한 줄로 알린다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InjectionFailure {
    pub extension: String,
    pub part: String,
    /// 어댑터나 도우미가 낸 원문이라 번역하지 않는다.
    pub reason: String,
}

impl InjectionFailure {
    fn of(part: &InjectedPart, reason: impl Into<String>) -> Self {
        Self {
            extension: part.extension.clone(),
            part: part.name.clone(),
            reason: reason.into(),
        }
    }
}

/// provider에 직접 설치된 항목 하나. 어댑터가 provider의 사용자 폴더를 읽기만 해서 만든다.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DirectInstall {
    pub(crate) kind: DirectKind,
    pub(crate) name: String,
    pub(crate) origin: DirectOrigin,
}

/// 직접 설치 항목의 원본 위치. 옮길 때만 읽는다.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum DirectOrigin {
    /// 스킬 폴더.
    Folder(PathBuf),
    /// 명령 프롬프트 파일.
    File(PathBuf),
    /// MCP 서버 정의. 확장의 `.mcp.json` 형식(`command`, `args`, `env`, `cwd`, `url`)이다. 값에 비밀이 들 수 있어
    /// 출력하지 않는다.
    Server(Value),
    /// 옮기지 않고 추적만 한다.
    TrackedOnly,
}

/// 옮겼을 때 확장 부분이 되는 종류. 플러그인은 부분이 아니라 `None`.
pub(crate) fn part_kind_of(kind: DirectKind) -> Option<ExtensionPartKind> {
    match kind {
        DirectKind::Skill => Some(ExtensionPartKind::Skill),
        DirectKind::Command => Some(ExtensionPartKind::Command),
        DirectKind::McpServer => Some(ExtensionPartKind::McpServer),
        DirectKind::Plugin => None,
    }
}

/// 한 provider에서 읽는 직접 설치 항목 수의 한도(초안). 폴더가 비정상적으로 커도 시작이 느려지지 않게 한다.
pub(crate) const DIRECT_INSTALL_LIMIT: usize = 500;

// cost: time O(n), heap O(n), stack O(1), io n
// vars: n = 폴더 항목 수
/// `dir/<이름>/SKILL.md`가 있는 폴더마다 스킬 하나. 이름이 `.`으로 시작하는 폴더(provider 내장)와 링크는 건너뛴다.
pub(crate) fn scan_skill_folders(dir: &Path) -> Vec<DirectInstall> {
    scan_entries(dir, |path, name| {
        (path.is_dir() && path.join("SKILL.md").is_file()).then(|| DirectInstall {
            kind: DirectKind::Skill,
            name: name.to_owned(),
            origin: DirectOrigin::Folder(path.to_path_buf()),
        })
    })
}

// cost: time O(n), heap O(n), stack O(1), io n
// vars: n = 폴더 항목 수
/// `dir/<이름>.<extension>` 파일마다 명령 하나. 이름이 `.`으로 시작하는 파일과 링크는 건너뛴다.
pub(crate) fn scan_command_files(dir: &Path, extension: &str) -> Vec<DirectInstall> {
    scan_entries(dir, |path, file| {
        let name = file.strip_suffix(&format!(".{extension}"))?;
        path.is_file().then(|| DirectInstall {
            kind: DirectKind::Command,
            name: name.to_owned(),
            origin: DirectOrigin::File(path.to_path_buf()),
        })
    })
}

fn scan_entries(
    dir: &Path,
    pick: impl Fn(&Path, &str) -> Option<DirectInstall>,
) -> Vec<DirectInstall> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<DirectInstall> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| !kind.is_symlink()))
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            if name.starts_with('.') {
                return None;
            }
            pick(&entry.path(), &name)
        })
        .collect();
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found.truncate(DIRECT_INSTALL_LIMIT);
    found
}

/// 어댑터가 주입할 확장 부분과, 같은 부분이면 같은 값인 지문. 지문이 같은 연결은 같은 주입 결과를 쓴다.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ExtensionInput<'a> {
    pub(crate) parts: &'a [InjectedPart],
    pub(crate) fingerprint: &'a str,
}

/// 정의 파일에서 읽은 MCP 서버나 훅 하나. `value`는 provider 형식으로 바꾸기 전 원본 JSON이다.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Definition {
    pub(crate) extension: String,
    pub(crate) name: String,
    pub(crate) value: Value,
}

/// 읽은 정의와 읽지 못한 부분.
#[derive(Debug, Default)]
pub(crate) struct Definitions {
    pub(crate) servers: Vec<Definition>,
    pub(crate) hooks: Vec<Definition>,
    pub(crate) failures: Vec<InjectionFailure>,
}

// cost: time O(p + f), heap O(f), stack O(1), io p
// vars: p = 정의 부분 수, f = 정의 파일 크기
// basis: estimate
/// MCP 서버와 훅 부분의 정의를 파일에서 읽는다. 정의를 읽지 못하거나 같은 MCP 서버 이름이 이미 있으면 그 부분만 실패로
/// 돌려주고 먼저 읽은 것을 남긴다. 훅의 이름은 이벤트 이름이라 확장끼리 겹쳐도 모두
/// 남긴다. 종류가 `layout`이 받는 것이 아니어도 실패로 돌려준다.
pub(crate) fn collect_definitions(layout: &ExtensionLayout, parts: &[InjectedPart]) -> Definitions {
    let mut found = Definitions::default();
    for part in parts {
        let (section, taken) = match part.kind {
            ExtensionPartKind::McpServer => ("mcpServers", &mut found.servers),
            ExtensionPartKind::Hook => ("hooks", &mut found.hooks),
            ExtensionPartKind::Skill | ExtensionPartKind::Command => continue,
        };
        let read = if !layout.accepts(part.kind) {
            Err("this provider does not take this kind of part".to_owned())
        } else if part.kind == ExtensionPartKind::McpServer
            && taken.iter().any(|known| known.name == part.name)
        {
            Err("a part with this name is already injected".to_owned())
        } else {
            read_definition(part, section)
        };
        match read {
            Ok(value) => taken.push(Definition {
                extension: part.extension.clone(),
                name: part.name.clone(),
                value,
            }),
            Err(reason) => found.failures.push(InjectionFailure::of(part, reason)),
        }
    }
    found
}

fn read_definition(part: &InjectedPart, section: &str) -> Result<Value, String> {
    let text = std::fs::read_to_string(&part.source).map_err(|error| error.to_string())?;
    let file: Value = serde_json::from_str(&text).map_err(|error| error.to_string())?;
    file.get(section)
        .and_then(|all| all.get(&part.name))
        .cloned()
        .ok_or_else(|| "the part is missing from its definition file".to_owned())
}

// cost: time O(f), heap O(1), stack O(d), io f
// vars: f = 복사할 파일 수, d = 폴더 깊이
// basis: estimate
/// 스킬은 `<root>/<skills_dir>/<이름>/`, 명령은 `<root>/<폴더>/<이름>.<확장자>`로 복사한다. 이름이 겹치면 먼저 놓은
/// 것을 남기고 나중 것을 실패로 돌려준다. 파일을 놓은 부분이 있었는지는 `placed`로 알린다. 정의 부분은 건드리지 않는다.
pub(crate) fn place_files(root: &Path, layout: &ExtensionLayout, parts: &[InjectedPart]) -> Placed {
    let mut placed = Placed::default();
    for part in parts {
        let result = match (part.kind, layout.skills_dir, layout.commands) {
            (ExtensionPartKind::Skill, Some(dir), _) => place_skill(&root.join(dir), part),
            (ExtensionPartKind::Command, _, Some((dir, extension))) => {
                place_command(&root.join(dir), extension, part)
            }
            (ExtensionPartKind::Skill | ExtensionPartKind::Command, ..) => {
                Err("this provider does not take this kind of part".to_owned())
            }
            _ => continue,
        };
        match result {
            Ok(()) => placed.any = true,
            Err(reason) => placed.failures.push(InjectionFailure::of(part, reason)),
        }
    }
    placed
}

/// `place_files`의 결과.
#[derive(Debug, Default)]
pub(crate) struct Placed {
    /// 파일을 하나라도 놓았다.
    pub(crate) any: bool,
    pub(crate) failures: Vec<InjectionFailure>,
}

fn place_skill(dir: &Path, part: &InjectedPart) -> Result<(), String> {
    let dest = dir.join(&part.name);
    if dest.exists() {
        return Err("a skill with this name is already injected".to_owned());
    }
    std::fs::create_dir_all(&dest).map_err(|error| error.to_string())?;
    copy_into(&part.source, &dest, &mut 0).map_err(|error| error.to_string())
}

fn place_command(dir: &Path, extension: &str, part: &InjectedPart) -> Result<(), String> {
    let dest = dir.join(format!("{}.{extension}", part.name));
    if dest.exists() {
        return Err("a command with this name is already injected".to_owned());
    }
    std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    std::fs::copy(&part.source, &dest)
        .map(|_| ())
        .map_err(|error| error.to_string())
}
