//! 시작 화면. 로고, Saturn 버전, provider 버전, judge와 judge 버전, 폴더.
//! 설계: docs/design/tui.md

use std::path::PathBuf;

use ratatui::Frame;
use ratatui::layout::Rect;
use saturn_protocol::ids::Provider;

use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use saturn_protocol::rpc::Notification;

use crate::i18n::{self, Lang};
use crate::view::{EMPHASIS, centered, text_width};

pub const LOGO: &[&str] = &["saturn"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartInfo {
    pub saturn_version: String,
    /// 확인하지 못한 provider는 버전이 `None`.
    pub providers: Vec<(Provider, Option<String>)>,
    pub judge: Option<String>,
    pub judge_version: Option<String>,
    /// 채팅의 기본 폴더. 다른 폴더의 채팅을 이어 열었으면 그 채팅의 폴더다.
    pub folder: PathBuf,
    /// 더한 폴더.
    pub added_dirs: Vec<PathBuf>,
}

impl StartInfo {
    // cost: time O(m), heap O(m), stack O(1)
    // vars: m = 알림 글자 수
    // basis: estimate
    /// 빈 버전과 빈 judge 이름은 확인하지 못한 것으로 본다.
    pub fn from_notification(notification: &Notification) -> Option<Self> {
        let Notification::StartInfo {
            saturn_version,
            providers,
            judge,
            judge_version,
            folder,
            added_dirs,
        } = notification
        else {
            return None;
        };
        let non_empty = |value: &String| (!value.is_empty()).then(|| value.clone());
        Some(Self {
            saturn_version: saturn_version.clone(),
            providers: providers
                .iter()
                .map(|(provider, version)| (*provider, non_empty(version)))
                .collect(),
            judge: non_empty(judge),
            judge_version: non_empty(judge_version),
            folder: PathBuf::from(folder),
            added_dirs: added_dirs.iter().map(PathBuf::from).collect(),
        })
    }

    // cost: time O(m), heap O(m), stack O(1)
    // vars: m = 정보 글자 수
    // basis: estimate
    /// 머리 셀과 plain 출력이 함께 쓴다.
    pub fn lines(&self, lang: Lang) -> Vec<String> {
        let mut lines: Vec<String> = LOGO.iter().map(|line| line.to_string()).collect();
        lines.push(format!("Saturn {}", self.saturn_version));
        lines.extend(self.providers.iter().map(|(provider, version)| {
            let version = version
                .clone()
                .unwrap_or_else(|| lang.tr(i18n::VERSION_UNKNOWN).to_string());
            format!("{} {version}", i18n::provider_name(*provider))
        }));
        let judge = match (&self.judge, &self.judge_version) {
            (Some(judge), Some(version)) => format!("{judge} · {version}"),
            (Some(judge), None) => judge.clone(),
            (None, _) => "-".to_string(),
        };
        lines.push(format!("{} {judge}", lang.tr(i18n::START_JUDGE)));
        lines.push(format!(
            "{} {}",
            lang.tr(i18n::START_FOLDER),
            self.folder.display()
        ));
        if !self.added_dirs.is_empty() {
            let dirs: Vec<String> = self
                .added_dirs
                .iter()
                .map(|dir| dir.display().to_string())
                .collect();
            lines.push(format!(
                "{} {}",
                lang.tr(i18n::START_ADDED_DIRS),
                dirs.join(", ")
            ));
        }
        lines
    }
}

#[derive(Debug)]
pub struct StartScreenView<'a> {
    pub info: &'a StartInfo,
    pub lang: Lang,
}

impl StartScreenView<'_> {
    // cost: time O(m), heap O(m), stack O(1)
    // vars: m = 정보 글자 수
    // basis: estimate
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let lines = self.info.lines(self.lang);
        let height = u16::try_from(lines.len()).unwrap_or(u16::MAX);
        let width = lines.iter().map(|line| text_width(line)).max().unwrap_or(0);
        let rect = centered(area, u16::try_from(width).unwrap_or(u16::MAX), height);
        let rows: Vec<Line> = lines
            .into_iter()
            .enumerate()
            .map(|(i, line)| {
                if i < LOGO.len() {
                    Line::from(Span::styled(line, EMPHASIS))
                } else {
                    Line::from(line)
                }
            })
            .collect();
        frame.render_widget(Paragraph::new(rows), rect);
    }
}

#[cfg(test)]
mod tests {
    use saturn_protocol::ids::Provider;

    use super::*;

    fn info() -> StartInfo {
        StartInfo {
            saturn_version: "0.1.0".to_string(),
            providers: vec![
                (Provider::Codex, Some("0.40.0".to_string())),
                (Provider::Claude, None),
            ],
            judge: Some("remote".to_string()),
            judge_version: Some("v3".to_string()),
            folder: PathBuf::from("/work/app"),
            added_dirs: Vec::new(),
        }
    }

    #[test]
    fn lines_show_the_chat_folder_and_the_added_folders() {
        let mut info = info();
        info.folder = PathBuf::from("/other/project");
        info.added_dirs = vec![PathBuf::from("/shared/lib"), PathBuf::from("/docs")];

        let lines = info.lines(Lang::Ko);

        assert_eq!(
            lines[lines.len() - 2..],
            ["폴더 /other/project", "더한 폴더 /shared/lib, /docs"]
        );
        assert_eq!(
            info.lines(Lang::En).last().map(String::as_str),
            Some("added folders /shared/lib, /docs")
        );
    }

    #[test]
    fn lines_list_versions_judge_and_folder() {
        assert_eq!(
            info().lines(Lang::Ko),
            vec![
                "saturn",
                "Saturn 0.1.0",
                "codex 0.40.0",
                "claude 확인 안 됨",
                "판단기 remote · v3",
                "폴더 /work/app",
            ]
        );
    }

    #[test]
    fn from_notification_treats_empty_values_as_unknown() {
        let notification = Notification::StartInfo {
            saturn_version: "0.1.0".to_string(),
            providers: vec![(Provider::Codex, String::new())],
            judge: String::new(),
            judge_version: String::new(),
            folder: "/w".to_string(),
            added_dirs: vec!["/x".to_string()],
        };

        let info = StartInfo::from_notification(&notification).unwrap();

        assert_eq!(info.providers, vec![(Provider::Codex, None)]);
        assert_eq!(info.judge, None);
        assert_eq!(info.added_dirs, vec![PathBuf::from("/x")]);
    }
}
