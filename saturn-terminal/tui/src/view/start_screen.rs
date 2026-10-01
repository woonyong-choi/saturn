//! 시작 화면. 로고, Saturn 버전, provider 버전, judge와 judge 버전, 폴더.
//!
//! 설계: docs/design/tui.md(영역 시작 화면). 실행 때 보이고, 첫 결과가 오면 대화 기록 맨 위 머리 셀로 바뀐다.
//! 값은 `Notification::StartInfo`로 받는다.

use std::path::PathBuf;

use ratatui::Frame;
use ratatui::layout::Rect;
use saturn_protocol::ids::Provider;

use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use saturn_protocol::rpc::Notification;

use crate::i18n::{self, Lang};
use crate::view::{EMPHASIS, centered, text_width};

/// 로고 줄.
pub const LOGO: &[&str] = &["saturn"];

/// 시작 화면 정보.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartInfo {
    /// Saturn 버전(`CARGO_PKG_VERSION`).
    pub saturn_version: String,
    /// provider와 버전. 확인하지 못한 provider는 버전이 `None`.
    pub providers: Vec<(Provider, Option<String>)>,
    /// judge 이름. 없으면 `None`.
    pub judge: Option<String>,
    /// judge 버전.
    pub judge_version: Option<String>,
    /// 작업 폴더.
    pub folder: PathBuf,
}

impl StartInfo {
    // cost: time O(m), heap O(m), stack O(1)
    // vars: m = 알림 글자 수
    // basis: estimate
    /// `Notification::StartInfo`에서 만든다. 빈 버전과 빈 judge 이름은 확인하지 못한 것으로 본다. 다른 알림이면 `None`.
    pub fn from_notification(notification: &Notification) -> Option<Self> {
        let Notification::StartInfo {
            saturn_version,
            providers,
            judge,
            judge_version,
            folder,
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
        })
    }

    // cost: time O(m), heap O(m), stack O(1)
    // vars: m = 정보 글자 수
    // basis: estimate
    /// 머리 셀과 plain 출력의 글 줄. 로고 다음 `Saturn 0.1.0`, provider별 한 줄, judge 한 줄, 폴더 한 줄.
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
        lines
    }
}

/// 시작 화면 그리기.
#[derive(Debug)]
pub struct StartScreenView<'a> {
    /// 정보.
    pub info: &'a StartInfo,
    /// 화면 언어.
    pub lang: Lang,
}

impl StartScreenView<'_> {
    // cost: time O(m), heap O(m), stack O(1)
    // vars: m = 정보 글자 수
    // basis: estimate
    /// 대화 기록 칸 가운데에 로고와 `StartInfo::lines`를 그린다.
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
        }
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
        };

        let info = StartInfo::from_notification(&notification).unwrap();

        assert_eq!(info.providers, vec![(Provider::Codex, None)]);
        assert_eq!(info.judge, None);
    }
}
