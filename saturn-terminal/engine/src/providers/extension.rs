//! 확장 주입의 공통 부분: 어댑터가 설명자에 적는 주입 형식, 주입할 부분과 실패 값, 파일 배치와 정의 읽기 도우미.
//! 어댑터는 이 도우미로 배치한 뒤 provider 형식으로 바꾸는 일(설정 파일 쓰기, 실행 인자)만 한다.
//! 설계: docs/design/extensions.md#주입

use std::path::{Path, PathBuf};

use saturn_protocol::rpc::ExtensionPartKind;
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
/// MCP 서버와 훅 부분의 정의를 파일에서 읽는다. 정의를 읽지 못하거나 같은 이름이 이미 있으면 그 부분만 실패로 돌려주고
/// 먼저 읽은 것을 남긴다. 종류가 `layout`이 받는 것이 아니어도 실패로 돌려준다.
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
        } else if taken.iter().any(|known| known.name == part.name) {
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
