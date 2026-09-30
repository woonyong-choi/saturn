//! 사용량 화면(`/usage`). 범위 `chat`, `today`, `week`, `all`.
//!
//! 설계: docs/design/tui.md(영역 사용량 화면, 키 사용량 화면), docs/design/providers-and-sessions.md(사용량 보고).
//! 보고하지 않은 값은 0으로 채우지 않고 `-`로 보인다.
//! TODO(#46): `Request::Usage`의 응답 알림이 protocol에 없다

use ratatui::Frame;
use ratatui::layout::Rect;
use saturn_protocol::ids::Provider;
use saturn_protocol::rpc::UsageRange;

use crate::i18n::Lang;

/// 에이전트 한 행.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentUsage {
    /// 에이전트 이름(이름표나 메인·보조 구분).
    pub agent: String,
    /// provider.
    pub provider: Provider,
    /// 새 입력 토큰.
    pub input: Option<u64>,
    /// 캐시 읽기 토큰.
    pub cache_read: Option<u64>,
    /// 캐시 쓰기 토큰.
    pub cache_write: Option<u64>,
    /// 출력 토큰.
    pub output: Option<u64>,
    /// 추론 토큰.
    pub reasoning: Option<u64>,
}

/// 사용량 화면 내용.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageTable {
    /// 범위.
    pub range: UsageRange,
    /// 에이전트별 행.
    pub agents: Vec<AgentUsage>,
    /// judge 호출 수.
    pub judge_calls: u32,
    /// judge 예상 비용(원문 문자열, 통화 포함).
    pub judge_cost: Option<String>,
    /// judge 실제 모델(`Enter` 상세).
    pub judge_models: Vec<String>,
    /// 맥락 정리 횟수.
    pub compactions: u32,
    /// 채점 수.
    pub gradings: u32,
}

/// 사용량 화면 상태.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageScreen {
    /// 받은 내용. 응답 전이면 `None`.
    pub table: Option<UsageTable>,
    /// 요청한 범위.
    pub range: UsageRange,
    /// `Enter`로 judge 실제 모델 상세를 펼쳤다.
    pub detail: bool,
}

impl UsageScreen {
    /// 범위로 연다(응답 대기).
    pub fn new(range: UsageRange) -> Self {
        todo!("#92")
    }

    /// `Enter` judge 실제 모델 상세를 펼치거나 접는다.
    pub fn toggle_detail(&mut self) {
        todo!("#92")
    }
}

/// 사용량 화면 그리기.
#[derive(Debug)]
pub struct UsageView<'a> {
    /// 화면 상태.
    pub screen: &'a UsageScreen,
    /// 화면 언어.
    pub lang: Lang,
}

impl UsageView<'_> {
    /// 범위 머리, 에이전트별 새 입력·캐시 읽기·캐시 쓰기·출력·추론 표(천 단위 쉼표, 미보고 `-`),
    /// judge 호출과 예상 비용, 맥락 정리, 채점 줄, 상세면 judge 실제 모델 목록을 그린다.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        todo!("#92")
    }
}
