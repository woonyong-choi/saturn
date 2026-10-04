//! 키 해석 계층. TUI는 키를 직접 보지 않고 `Keymap::resolve`가 돌려준 `Action`만 받는다.
//! 설계: docs/design/tui.md
//!
//! 구조는 세 층이다. 기본 표(`base`)는 동작별 키 표이고, 프리셋(`presets`)은 기본 표에서 다른 키만 덮어쓰는 표이며,
//! `Keymap`은 둘을 합쳐 키를 동작으로 바꾸는 해석기다. 새 프리셋은 표 하나와 `PRESETS` 등록 한 줄이다.

mod base;
mod presets;
#[cfg(test)]
mod tests;

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::keys::{Action, KeyArea, KeyContext};
use crate::view::popup::PopupKind;

pub(crate) use presets::PRESETS;

/// 시험에서 `saturn` 묶음으로 키 하나를 해석한다.
#[cfg(test)]
pub(crate) fn resolve_in_tests(area: KeyArea, key: KeyEvent) -> Option<Action> {
    Keymap::saturn()
        .resolve(area, None, key, KeyContext::default(), Instant::now())
        .map(|found| found.action)
}

/// 두 키를 이어 누르는 조합(`esc esc`)에서 첫 키와 둘째 키 사이의 최대 간격.
pub(crate) const SEQUENCE_WINDOW: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum KeymapError {
    #[error("unknown keymap preset: {name}")]
    UnknownPreset { name: String },
    #[error("unreadable key `{spec}`")]
    BadKey { spec: String },
    #[error("key `{key}` is bound to two actions in {scope:?}: {first:?} and {second:?}")]
    Conflict {
        scope: KeyArea,
        key: String,
        first: Action,
        second: Action,
    },
}

/// 정규화한 키 하나. `Shift`는 글자 키에서 떼고(`A`와 `a`는 다른 글자), `Shift+Tab`은 `BackTab`으로 모은다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Chord {
    code: KeyCode,
    mods: KeyModifiers,
}

impl Chord {
    fn new(code: KeyCode, mods: KeyModifiers) -> Self {
        match code {
            KeyCode::Tab | KeyCode::BackTab
                if code == KeyCode::BackTab || mods.contains(KeyModifiers::SHIFT) =>
            {
                Self {
                    code: KeyCode::BackTab,
                    mods: mods - KeyModifiers::SHIFT,
                }
            }
            KeyCode::Char(_) => Self {
                code,
                mods: mods - KeyModifiers::SHIFT,
            },
            _ => Self { code, mods },
        }
    }

    pub(crate) fn of(key: KeyEvent) -> Self {
        Self::new(key.code, key.modifiers)
    }

    /// `ctrl+j`, `shift+tab`, `f3`, `esc`처럼 `+`로 이은 소문자 표기.
    ///
    /// # Errors
    /// 모르는 이름이나 수정 키면 `BadKey`.
    pub(crate) fn parse(spec: &str) -> Result<Self, KeymapError> {
        let bad = || KeymapError::BadKey {
            spec: spec.to_owned(),
        };
        let mut parts: Vec<&str> = spec.split('+').collect();
        let name = parts
            .pop()
            .filter(|name| !name.is_empty())
            .ok_or_else(bad)?;
        let mut mods = KeyModifiers::NONE;
        for part in parts {
            mods |= match part {
                "ctrl" => KeyModifiers::CONTROL,
                "alt" => KeyModifiers::ALT,
                "shift" => KeyModifiers::SHIFT,
                _ => return Err(bad()),
            };
        }
        let code = match name {
            "enter" => KeyCode::Enter,
            "esc" => KeyCode::Esc,
            "tab" => KeyCode::Tab,
            "space" => KeyCode::Char(' '),
            "backspace" => KeyCode::Backspace,
            "up" => KeyCode::Up,
            "down" => KeyCode::Down,
            "left" => KeyCode::Left,
            "right" => KeyCode::Right,
            "home" => KeyCode::Home,
            "end" => KeyCode::End,
            _ => match name.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
                Some(number) if (1..=12).contains(&number) => KeyCode::F(number),
                _ => {
                    let mut chars = name.chars();
                    match (chars.next(), chars.next()) {
                        (Some(c), None) => KeyCode::Char(c),
                        _ => return Err(bad()),
                    }
                }
            },
        };
        Ok(Self::new(code, mods))
    }
}

/// 키가 이 동작이 되는 조건. 조건이 있는 줄이 `Always`보다 먼저 맞는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cond {
    Always,
    Running,
    ComposerEmpty,
    /// 입력창이 비었거나 불러온 기록 그대로다.
    HistoryBrowsable,
    /// 단순 방식으로 그리는 중이다.
    Plain,
}

impl Cond {
    fn holds(self, ctx: KeyContext) -> bool {
        match self {
            Self::Always => true,
            Self::Running => ctx.running,
            Self::ComposerEmpty => ctx.composer_empty,
            Self::HistoryBrowsable => ctx.history_browsable,
            Self::Plain => ctx.plain,
        }
    }
}

/// 동작별 키 표의 한 줄. `keys`의 각 항목은 키 하나(`ctrl+j`)나 이어 누르는 두 키(`esc esc`)다.
/// 키가 없는 줄은 `command`로만 한다.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Binding {
    pub action: Action,
    pub scopes: Vec<KeyArea>,
    pub keys: Vec<&'static str>,
    pub when: Cond,
    /// 같은 동작을 하는 `/` 명령.
    pub command: Option<&'static str>,
}

impl Binding {
    pub(crate) fn new(action: Action, scopes: &[KeyArea], keys: &[&'static str]) -> Self {
        Self {
            action,
            scopes: scopes.to_vec(),
            keys: keys.to_vec(),
            when: Cond::Always,
            command: None,
        }
    }

    pub(crate) fn when(mut self, when: Cond) -> Self {
        self.when = when;
        self
    }

    pub(crate) fn command(mut self, command: &'static str) -> Self {
        self.command = Some(command);
        self
    }
}

/// 해석 결과. `scope`는 동작을 찾은 표의 영역이다. 영역이 입력창 표로 넘긴 키는 `Composer`다.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Resolved {
    pub action: Action,
    pub scope: KeyArea,
}

#[derive(Debug, Clone)]
struct Entry {
    scope: KeyArea,
    sequence: Vec<Chord>,
    when: Cond,
    action: Action,
}

#[derive(Debug, Clone)]
pub(crate) struct Keymap {
    name: &'static str,
    bindings: Vec<Binding>,
    entries: Vec<Entry>,
    previous: Option<(Chord, Instant)>,
}

impl Keymap {
    /// 기본 표에 프리셋의 덮어쓰기를 얹는다.
    ///
    /// # Errors
    /// 모르는 프리셋이면 `UnknownPreset`, 읽을 수 없는 키나 한 영역 안의 겹치는 키면 `BadKey`, `Conflict`.
    pub(crate) fn load(name: &str) -> Result<Self, KeymapError> {
        let preset = PRESETS
            .iter()
            .find(|preset| preset.name == name)
            .ok_or_else(|| KeymapError::UnknownPreset {
                name: name.to_owned(),
            })?;
        Self::from_tables(preset.name, base::table(), (preset.overrides)())
    }

    /// 기본 설정인 `saturn` 묶음. 표가 틀리면 시험이 먼저 잡는다.
    pub(crate) fn saturn() -> Self {
        Self::load("saturn").expect("the saturn keymap should be consistent")
    }

    pub(crate) fn from_tables(
        name: &'static str,
        base: Vec<Binding>,
        overrides: Vec<Binding>,
    ) -> Result<Self, KeymapError> {
        let bindings = inherit(base, overrides);
        let entries = entries(&bindings)?;
        Ok(Self {
            name,
            bindings,
            entries,
            previous: None,
        })
    }

    pub(crate) fn name(&self) -> &'static str {
        self.name
    }

    /// 상속을 거친 최종 표.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "표 검사와 도움말 생성이 읽는다")
    )]
    pub(crate) fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    // cost: time O(e), heap O(1), stack O(1)
    // vars: e = 표 줄 수
    // basis: estimate
    /// 전역 표, `area`의 표, `fallback`의 표, 마지막 표 순서로 찾고, 표에 없는 글자 키는 `area`의 입력 규칙을 따른다.
    /// 이어 누르는 조합은 앞 키가 `SEQUENCE_WINDOW` 안에 눌렸을 때만 맞는다.
    pub(crate) fn resolve(
        &mut self,
        area: KeyArea,
        fallback: Option<KeyArea>,
        key: KeyEvent,
        ctx: KeyContext,
        now: Instant,
    ) -> Option<Resolved> {
        let chord = Chord::of(key);
        let previous = self
            .previous
            .take()
            .filter(|(_, at)| now.saturating_duration_since(*at) <= SEQUENCE_WINDOW)
            .map(|(chord, _)| chord);
        let scopes: Vec<KeyArea> = [
            Some(KeyArea::Global),
            Some(area),
            fallback,
            Some(KeyArea::Fallback),
        ]
        .into_iter()
        .flatten()
        .collect();
        if let Some(previous) = previous
            && let Some(found) = self.lookup(&scopes, &[previous, chord], ctx)
        {
            return Some(found);
        }
        self.previous = Some((chord, now));
        self.lookup(&scopes, &[chord], ctx)
            .or_else(|| typed(area, area, key, ctx))
            .or_else(|| fallback.and_then(|scope| typed(scope, scope, key, ctx)))
    }

    fn lookup(&self, scopes: &[KeyArea], sequence: &[Chord], ctx: KeyContext) -> Option<Resolved> {
        scopes.iter().find_map(|scope| {
            self.entries
                .iter()
                .find(|entry| {
                    entry.scope == *scope && entry.sequence == sequence && entry.when.holds(ctx)
                })
                .map(|entry| Resolved {
                    action: entry.action.clone(),
                    scope: *scope,
                })
        })
    }
}

/// 덮어쓰기 줄마다 기본 표에서 같은 동작과 조건의 줄이 가진 그 영역들을 걷어내고 덮어쓰기 줄을 더한다.
/// 키가 없는 덮어쓰기 줄은 그 동작의 키를 걷기만 하고, 명령은 기본 표의 것을 물려받는다.
fn inherit(base: Vec<Binding>, overrides: Vec<Binding>) -> Vec<Binding> {
    let mut table = base;
    for mut over in overrides {
        for line in table
            .iter_mut()
            .filter(|line| line.action == over.action && line.when == over.when)
        {
            if over.command.is_none() {
                over.command = line.command;
            }
            line.scopes.retain(|scope| !over.scopes.contains(scope));
        }
        table.retain(|line| !line.scopes.is_empty());
        table.push(over);
    }
    table
}

fn entries(bindings: &[Binding]) -> Result<Vec<Entry>, KeymapError> {
    let mut entries: Vec<Entry> = Vec::new();
    for binding in bindings {
        for spec in &binding.keys {
            let sequence = spec
                .split(' ')
                .map(Chord::parse)
                .collect::<Result<Vec<_>, _>>()?;
            for scope in &binding.scopes {
                if let Some(existing) = entries.iter().find(|entry| {
                    entry.scope == *scope
                        && entry.sequence == sequence
                        && entry.when == binding.when
                        && entry.action != binding.action
                }) {
                    return Err(KeymapError::Conflict {
                        scope: *scope,
                        key: (*spec).to_owned(),
                        first: existing.action.clone(),
                        second: binding.action.clone(),
                    });
                }
                entries.push(Entry {
                    scope: *scope,
                    sequence: sequence.clone(),
                    when: binding.when,
                    action: binding.action.clone(),
                });
            }
        }
    }
    entries.sort_by_key(|entry| entry.when == Cond::Always);
    Ok(entries)
}

/// 글자 키가 표에 없을 때의 입력 규칙. 영역마다 다르고 키 표로 바꿀 수 없다.
fn typed(area: KeyArea, scope: KeyArea, key: KeyEvent, ctx: KeyContext) -> Option<Resolved> {
    let KeyCode::Char(c) = key.code else {
        return None;
    };
    if !(key.modifiers - KeyModifiers::SHIFT).is_empty() {
        return None;
    }
    let action = match area {
        KeyArea::Composer => composer_char(c, ctx),
        KeyArea::Search | KeyArea::RouterKeyPrompt | KeyArea::Input | KeyArea::TaskListEdit => {
            Action::Insert(c)
        }
        _ => return None,
    };
    Some(Resolved { action, scope })
}

fn composer_char(c: char, ctx: KeyContext) -> Action {
    match c {
        '$' if ctx.at_word_start => Action::OpenPopup(PopupKind::Skill),
        '/' => Action::OpenPopup(PopupKind::Command),
        '@' => Action::OpenPopup(PopupKind::File),
        '?' if ctx.composer_empty => Action::ShowShortcuts,
        _ => Action::Insert(c),
    }
}
