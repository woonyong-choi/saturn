//! 화면 문구 언어. 운영체제 언어로 영어와 한국어 중 하나를 고른다.
//!
//! 문구 키는 설계 표(docs/design/tui.md 상태 표시)의 한국어 원문이다. `Lang::Ko`면 키를 그대로, `Lang::En`이면 번역표에서 찾는다.
//! 값이 들어가는 문구(`[A] codex · 45초 · Token 3,210`)는 아래 조각 상수와 `format_*` 함수를 이어 만든다.
//! 전체 화면과 plain 출력은 같은 상수와 함수를 써서 같은 결과를 낸다.

use std::time::Duration;

use saturn_protocol::ids::Provider;

// 상태판 실행 줄
/// 출력이 아직 없는 실행 중 작업(`⠙ [A] 작업 중`).
pub const WORKING: &str = "작업 중";
/// 실행 줄 하는 일: provider가 생각하는 중.
pub const THINKING: &str = "생각 중";
/// 실행 줄 하는 일: 파일 조회.
pub const READING_FILE: &str = "파일 읽는 중";
/// 실행 줄 하는 일: 파일 수정.
pub const EDITING_FILE: &str = "파일 수정 중";
/// 실행 줄 하는 일: 명령 실행. 뒤에 명령 앞 40칸을 붙인다.
pub const RUNNING_COMMAND: &str = "명령 실행 중";
/// 실행 줄 하는 일: 허가 응답 대기. 경과 시간이 멈춘다.
pub const AWAITING_PERMISSION: &str = "허가 기다림";
/// 실행 줄 하는 일: 맥락 정리.
pub const COMPACTING: &str = "맥락 정리 중";
/// 실행 줄 하는 일: provider 전환.
pub const SWITCHING_PROVIDER: &str = "공급자 전환 중";
/// `하위 에이전트 2개 실행 중`의 앞. 뒤에 `{n}개 실행 중`.
pub const SUBAGENTS: &str = "하위 에이전트";
/// `{n}개 실행 중`의 뒤.
pub const RUNNING_COUNT_SUFFIX: &str = "개 실행 중";
/// 사용량 보고 전 토큰 칸.
pub const TOKEN_UNREPORTED: &str = "Token -";
/// 토큰 칸 머리. `Token 3,210`.
pub const TOKEN: &str = "Token";

// 상태판 판단 줄, 학습 줄
/// judge가 입력을 판단하는 중(`⠹ [D] 판단 중`).
pub const JUDGING: &str = "판단 중";
/// `/train` 진행 줄 이름표(`⠼ [학습]`).
pub const TRAINING: &str = "학습";

// 상태판 대기 줄
/// 대기 줄 머리(`· [C] 대기 · ...`).
pub const QUEUED: &str = "대기";
/// `A 다음`의 뒤. 실행 중인 작업 뒤에 보낼 입력.
pub const AFTER_TASK_SUFFIX: &str = "다음";
/// 같은 채팅 앞 입력의 판단 차례. `[보내기]` 없이 `[취소]`만.
pub const JUDGE_ORDER: &str = "판단 차례";
/// judge 연결 회복을 기다린다.
pub const JUDGE_CONNECTION: &str = "판단기 연결 기다림";
/// 다른 에이전트의 쓰기 끝을 기다린다.
pub const WRITE_TURN: &str = "쓰기 차례";
/// session 교체 끝을 기다린다.
pub const AFTER_COMPACTION: &str = "맥락 정리 뒤";
/// 모든 작업 뒤에 실행할 session 변경 명령.
pub const AFTER_ALL_TASKS: &str = "모든 작업 뒤";
/// 대기 줄 버튼. `/send`와 같다.
pub const BUTTON_SEND: &str = "[보내기]";
/// 대기 줄·보류 줄 버튼. `/cancel`과 같다.
pub const BUTTON_CANCEL: &str = "[취소]";

// 상태판 보류 줄
/// 보류 줄 머리(`‖ [A] 보류`).
pub const HELD: &str = "보류";
/// 보류 줄 버튼. `/continue`와 같다.
pub const BUTTON_CONTINUE: &str = "[이어서]";
/// 중지 결과 머리(`‖ 멈춤 · [A] [C] 보류됨 · /continue 로 이어서`).
pub const STOPPED: &str = "멈춤";
/// 중지 결과 가운데.
pub const HELD_DONE: &str = "보류됨";
/// 중지 결과 끝.
pub const CONTINUE_HINT: &str = "/continue 로 이어서";
/// 멈춤 뒤 남은 프로세스(`멈춤 확인 안 됨 · N개 남음`)의 앞.
pub const STOP_UNCONFIRMED: &str = "멈춤 확인 안 됨";
/// `N개 남음`의 뒤.
pub const REMAINING_SUFFIX: &str = "개 남음";
/// 보류 닫기 확인(`‖ [E] 보류를 닫을까요?`). 보내지 않은 입력 취소, 수정된 파일 유지.
pub const CLOSE_HELD_QUESTION: &str = "보류를 닫을까요?";
/// 보류 종료 완료(`[E] 보류를 닫았습니다`).
pub const HELD_CLOSED: &str = "보류를 닫았습니다";

// 상태판 알림 줄
/// judge 호출 일시 실패, 질문별 대체 규칙 적용.
pub const JUDGE_PAUSED: &str = "자동 판단 일시 중단";
/// judge 호출 연속 3회 실패, 새 입력 접수 중지.
pub const INTAKE_STOPPED: &str = "새 입력 접수 중단 · 판단기 연결을 확인하세요";
/// 끼워 넣기 실측 전(`바로 반영: 준비 중 (codex)`). 뒤에 ` (provider)`.
pub const STEER_NOT_READY: &str = "바로 반영: 준비 중";
/// `[보내기]`를 눌렀으나 judge 실패, 차례에 전송.
pub const JUDGE_UNAVAILABLE_SEND: &str = "판단기 연결 없음 · 차례에 보냅니다";
/// 다른 Saturn 프로세스가 실행 중인 채팅. 작업 목록에서 읽기 전용.
pub const BUSY_ELSEWHERE: &str = "다른 Saturn에서 실행 중";
/// 폴더 설정 검사 실패(`폴더 설정 오류 · 이전 설정 번호 12로 계속 · 줄 7: ...`)의 앞.
pub const SETTINGS_ERROR: &str = "폴더 설정 오류";
/// `이전 설정 번호 12로 계속`의 앞. 뒤에 `{n}로 계속`.
pub const SETTINGS_PREVIOUS: &str = "이전 설정 번호";
/// `{n}로 계속`의 뒤.
pub const SETTINGS_CONTINUE_SUFFIX: &str = "로 계속";

// 입력 전달 상태
/// 에이전트로 입력 전달 중. 취소 불가.
pub const DELIVERING: &str = "전달 중";
/// 에이전트에 입력 전달 완료. 취소 불가.
pub const APPLIED: &str = "반영됨";

// 대화 기록
/// 작업 실패 머리 끝(`[A] codex · 45초 · 실패`). 다음 줄에 원인 한 줄.
pub const FAILED: &str = "실패";
/// 결과 불명(`[A] 결과 확인 필요 · /continue A`). 보류 줄과 함께.
pub const NEEDS_CHECK: &str = "결과 확인 필요";
/// 맥락 정리 뒤 같은 작업 계속(`[A] 맥락 정리 후 이어서 진행`).
pub const COMPACTED: &str = "맥락 정리 후 이어서 진행";
/// provider 전환(`[A] codex → claude로 전환`)의 끝.
pub const SWITCHED_SUFFIX: &str = "로 전환";
/// 모든 작업이 끝난 순간의 합계 머리(`이번 요청 · codex Token 4,120 · 판단기 3회 Token 9,870 · 2분 31초`).
pub const REQUEST_SUMMARY: &str = "이번 요청";
/// 합계의 judge 칸 머리(`판단기 3회`).
pub const JUDGE_CALLS: &str = "판단기";
/// `3회`의 뒤.
pub const TIMES_SUFFIX: &str = "회";
/// 피드백 질문 끼워 넣기 판단(`[A]에 이어서 보냈어요`)의 뒤.
pub const FEEDBACK_STEERED: &str = "에 이어서 보냈어요";
/// 피드백 질문 가운데.
pub const FEEDBACK_QUESTION: &str = "판단이 맞았나요? (선택)";
/// 피드백 선택지. 두 칸 띄어 이어 붙인다.
pub const FEEDBACK_CHOICES: &str = "1 맞아요  2 아니에요  0 닫기";
/// 아니에요 답 뒤 바로잡기 제안(`[B] 바로 새 작업으로 실행할까요? [실행] [그대로]`).
pub const CORRECTION_QUESTION: &str = "바로 새 작업으로 실행할까요?";
/// 바로잡기 제안 버튼.
pub const BUTTON_RUN: &str = "[실행]";
/// 바로잡기 제안 버튼.
pub const BUTTON_KEEP: &str = "[그대로]";
/// 채점할 판단 부족(`채점할 판단 83 / 200건 · 200건이 쌓이면 실행할 수 있습니다`)의 앞.
pub const TRAIN_SHORT: &str = "채점할 판단";
/// 위 문구의 끝. 앞에 `{need}건이 `.
pub const TRAIN_SHORT_SUFFIX: &str = "쌓이면 실행할 수 있습니다";
/// 1,000자를 넘는 붙여넣은 내용 요소(`[붙여넣은 내용 1,204자]`)의 앞.
pub const PASTED: &str = "붙여넣은 내용";
/// 글자 수 단위(`1,204자`).
pub const CHARS_SUFFIX: &str = "자";
/// 건수 단위(`200건`).
pub const COUNT_SUFFIX: &str = "건";

// 바닥줄
/// 바닥줄 왼쪽 키 안내.
pub const FOOTER_HINT: &str = "/help 도움말 · Ctrl+C 멈춤";
/// 맥락 크기 머리(`맥락 38K/200K`).
pub const CONTEXT: &str = "맥락";
/// 맥락 크기 측정 불가.
pub const CONTEXT_UNKNOWN: &str = "맥락 미확인";

// 창
/// 보류 재개 질문 선택지.
pub const RESUME_ALL: &str = "모두 이어서";
/// 보류 재개 질문 선택지. 보류 목록에서 골라 하나씩 `Continue`.
pub const RESUME_PICK: &str = "골라서 이어서";
/// 보류 재개 질문 선택지. 아무것도 보내지 않는다.
pub const RESUME_LEAVE: &str = "그대로 두기";
/// 허가 요청 창 `y`.
pub const PERMISSION_ALLOW: &str = "실행";
/// 허가 요청 창 `a`.
pub const PERMISSION_ALLOW_FOR_TASK: &str = "이 작업 동안 같은 명령 허용";
/// 허가 요청 창 `d`.
pub const PERMISSION_DENY: &str = "실행하지 않고 계속";
/// 허가 요청 창 `Esc`.
pub const PERMISSION_DENY_AND_REDIRECT: &str = "실행하지 않고 다르게 하라고 말하기";
/// 작업 목록 필터.
pub const FILTER_ALL: &str = "전체";
/// 작업 목록 필터.
pub const FILTER_NEEDS_CHECK: &str = "확인 필요";
/// 작업 목록 필터.
pub const FILTER_RUNNING: &str = "실행 중";
/// 작업 목록 필터.
pub const FILTER_QUEUED: &str = "대기";
/// 작업 목록 필터.
pub const FILTER_HELD: &str = "보류";
/// 작업 목록 필터.
pub const FILTER_DONE: &str = "끝남";
/// 작업 상세에서 provider의 모델 보고 없음.
pub const MODEL_UNREPORTED: &str = "모델 미보고";
/// 명령 목록 오른쪽 출처 표시(Saturn 명령).
pub const SOURCE_SATURN: &str = "Saturn";

/// 화면 언어.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    /// 한국어. 키 문구를 그대로 쓴다.
    Ko,
    /// 영어. 운영체제 언어가 한국어가 아니면 이것.
    #[default]
    En,
}

impl Lang {
    /// 운영체제 언어로 고른다. `LC_ALL` → `LC_MESSAGES` → `LANG` 순으로 처음 비어 있지 않은 값을 보고,
    /// `ko`로 시작하면 `Ko`, 그 밖(값 없음, `C`, `POSIX` 포함)은 `En`.
    pub fn detect() -> Self {
        todo!("#92")
    }

    /// 한국어 키 문구를 이 언어로 바꾼다. `Ko`면 그대로, `En`이면 `english`에서 찾고 없으면 키 그대로(누락은 테스트로 막는다).
    pub fn tr(self, ko: &'static str) -> &'static str {
        todo!("#92")
    }
}

/// 한국어 키의 영어 문구. 이 파일의 모든 문구 상수가 들어 있어야 한다(테스트로 확인).
fn english(ko: &str) -> Option<&'static str> {
    todo!("#92")
}

/// 천 단위 쉼표 숫자(`3,210`, `1,204`). 두 언어 같다.
pub fn format_count(n: u64) -> String {
    todo!("#92")
}

/// 경과 시간. `Ko`: 60초 미만 `45초`, 그 이상 `2분 31초`, 초가 0이면 `1분`. `En`: `45s`, `2m 31s`, `1m`. 초 미만은 버린다.
pub fn format_elapsed(lang: Lang, elapsed: Duration) -> String {
    todo!("#92")
}

/// 토큰 수를 천 단위 `K`로(`38K`, `200K`). 1,000 미만은 그대로, 나머지는 내림.
pub fn format_kilo(tokens: u64) -> String {
    todo!("#92")
}

/// provider 화면 이름. `Codex` → `codex`, `Claude` → `claude`. 두 언어 같다.
pub fn provider_name(provider: Provider) -> &'static str {
    todo!("#92")
}
