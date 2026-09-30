//! 화면 없는 plain 출력. 파이프와 CI에서 전체 화면 대신 쓴다.
//!
//! 설계: docs/design/tui.md(화면 언어와 출력 방식, 요구사항 같은 명령 같은 결과).
//! 같은 `state::ChatState`와 같은 문구 함수(`TranscriptCell::lines`, `status_board` 문구)를 써서
//! 전체 화면 대화 기록과 같은 줄을 stdout에 한 줄씩 쓴다. 스피너, 버튼, 틱 갱신 줄(경과 시간)은 쓰지 않는다.
//! 창이 필요한 상호작용(허가 요청, 폴더 신뢰, 보류 재개 질문)의 plain 처리는 설계에 없다.
//! TODO(#57): plain을 켜는 조건과 우선순위, 설정 키 이름

use std::io::Write;
use std::time::Instant;

use saturn_protocol::rpc::Notification;

use crate::i18n::Lang;
use crate::state::ChatState;

/// plain 출력기.
#[derive(Debug)]
pub struct PlainOutput<W: Write> {
    out: W,
    lang: Lang,
    chat: ChatState,
    finished: bool,
}

impl<W: Write> PlainOutput<W> {
    /// `out`(보통 stdout)에 쓰는 출력기.
    pub fn new(out: W, lang: Lang) -> Self {
        todo!("#92")
    }

    /// 제출한 원문을 기억한다(에코에 쓴다).
    pub fn submitted(&mut self, text: String) {
        todo!("#92")
    }

    /// 알림 하나를 반영하고 대화 기록에 생길 셀과 같은 줄을 쓴다.
    /// 에코 `> [A] 원문`, 결과 머리줄, 실패 원인, 한 줄 알림, 이번 요청 합계, 멈춤 결과, 알림 줄(처음 한 번),
    /// 허가 요청(요청 내용과 이유 한 줄). 모델 글은 완성된 줄마다 바로 쓴다.
    ///
    /// # Errors
    /// 쓰기 실패.
    pub fn apply(&mut self, notification: Notification, now: Instant) -> std::io::Result<()> {
        todo!("#92")
    }

    /// 모든 작업이 끝났다(`ChatNotice::RequestSummary`를 쓴 뒤 참). 표준 입력이 끝났고 이것이 참이면 `run_plain`이 끝난다.
    pub fn is_finished(&self) -> bool {
        todo!("#92")
    }

    /// 한 줄 쓰고 바로 비운다(파이프 버퍼에 머물지 않게).
    fn line(&mut self, text: &str) -> std::io::Result<()> {
        todo!("#92")
    }
}
