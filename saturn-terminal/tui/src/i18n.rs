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

// 아래 문구는 설계 표에 없어 정한 초안이다(docs/design/tui.md 초안 값).
// 대화 기록 초안
/// 새 작업 판단의 피드백 질문 머리(`[B] 새 작업으로 보냈어요`).
pub const FEEDBACK_NEW_TASK: &str = "새 작업으로 보냈어요";
/// 대기 판단의 피드백 질문 머리(`[C] 대기열에 넣었어요`).
pub const FEEDBACK_QUEUED: &str = "대기열에 넣었어요";
/// 셸 명령 셀의 종료 코드 줄(`종료 코드 1`).
pub const SHELL_EXIT: &str = "종료 코드";
/// 알 수 없는 명령 경고(`알 수 없는 명령: /foo`).
pub const COMMAND_UNKNOWN: &str = "알 수 없는 명령";
/// 잘못된 명령 인자 경고(`잘못된 인자: /usage year`).
pub const COMMAND_INVALID: &str = "잘못된 인자";
/// 셸 명령을 띄우지 못했을 때의 경고.
pub const SHELL_FAILED: &str = "셸 명령을 실행하지 못했습니다";
/// 개수 단위(`2개`).
pub const ITEMS_SUFFIX: &str = "개";

// 시작 화면 초안
/// judge 줄 머리(`판단기 remote · v3`). 값은 `JUDGE_CALLS`와 같다.
pub const START_JUDGE: &str = "판단기";
/// 폴더 줄 머리(`폴더 ~/src/app`).
pub const START_FOLDER: &str = "폴더";
/// 버전을 확인하지 못한 provider(`codex 확인 안 됨`).
pub const VERSION_UNKNOWN: &str = "확인 안 됨";

// 창 초안
/// judge 키 입력 창 제목.
pub const JUDGE_KEY_TITLE: &str = "판단기 키 입력";
/// judge 키 입력 창 안내.
pub const JUDGE_KEY_HINT: &str = "Enter 확인 · Esc 종료";
/// 폴더 설정 신뢰 창 제목.
pub const TRUST_TITLE: &str = "폴더 설정 신뢰";
/// 폴더 설정 신뢰 창 칸: 경로.
pub const TRUST_PATH: &str = "경로";
/// 폴더 설정 신뢰 창 칸: 지문.
pub const TRUST_FINGERPRINT: &str = "지문";
/// 폴더 설정 신뢰 창 칸: 적용되는 항목.
pub const TRUST_APPLIED: &str = "적용되는 항목";
/// 폴더 설정 신뢰 창 칸: 무시되는 항목.
pub const TRUST_IGNORED: &str = "무시되는 항목";
/// 폴더 설정 신뢰 창 칸: 바뀐 줄.
pub const TRUST_CHANGED: &str = "바뀐 줄";
/// 폴더 설정 신뢰 창 선택지 `1`.
pub const TRUST_APPLY: &str = "적용하고 계속";
/// 폴더 설정 신뢰 창 두 번째 선택지. 설계에 문구가 없다.
pub const TRUST_SKIP: &str = "적용하지 않고 계속";
/// 폴더 설정 신뢰 창 선택지 `3`.
pub const TRUST_QUIT: &str = "종료";
/// 보류 재개 질문 제목.
pub const RESUME_TITLE: &str = "보류된 작업이 있습니다";
/// `골라서 이어서` 단계의 확정 행.
pub const RESUME_CONFIRM: &str = "고른 작업 이어서";
/// 허가 요청 창 칸: 이유.
pub const PERMISSION_REASON: &str = "이유";
/// 허가 요청 창: 허가를 기다리는 다른 작업 수의 앞(`허가를 기다리는 다른 작업 2개`).
pub const PERMISSION_WAITING: &str = "허가를 기다리는 다른 작업";
/// `?` 단축키 안내 제목.
pub const SHORTCUTS_TITLE: &str = "단축키";
/// 작업 목록 화면 제목.
pub const TASKS_TITLE: &str = "작업 목록";
/// 작업 목록이 비었다.
pub const TASKS_EMPTY: &str = "작업 없음";
/// 작업 상세 칸: 모델.
pub const TASKS_MODEL: &str = "모델";
/// 작업 상세 칸: subagent와 자식 채팅 수.
pub const TASKS_CHILDREN: &str = "하위 항목";
/// 작업 목록 입력 줄: 검색.
pub const TASKS_SEARCH: &str = "검색";
/// 작업 목록 입력 줄: 새 이름.
pub const TASKS_RENAME: &str = "새 이름";
/// 작업 목록 입력 줄: 묶음.
pub const TASKS_GROUP: &str = "묶음";
/// 작업 목록 도움말.
pub const TASKS_HELP: &str = "Enter 이동 · Esc 닫기 · Tab 필터 · c 이어서 · d 취소 · f 검색 · g 묶음 · n 새 채팅 · r 이름 · s 보내기";
/// 응답을 기다리는 화면.
pub const LOADING: &str = "불러오는 중";
/// 사용량 화면 제목.
pub const USAGE_TITLE: &str = "사용량";
/// 사용량 표 칸: 대상.
pub const USAGE_WHO: &str = "대상";
/// 사용량 표 칸: 새 입력.
pub const USAGE_INPUT: &str = "새 입력";
/// 사용량 표 칸: 캐시 읽기.
pub const USAGE_CACHE_READ: &str = "캐시 읽기";
/// 사용량 표 칸: 캐시 쓰기.
pub const USAGE_CACHE_WRITE: &str = "캐시 쓰기";
/// 사용량 표 칸: 출력.
pub const USAGE_OUTPUT: &str = "출력";
/// 사용량 표 칸: 추론.
pub const USAGE_REASONING: &str = "추론";
/// 사용량 표 칸: judge 호출.
pub const USAGE_JUDGE_CALLS: &str = "판단기 호출";
/// 사용량 표 칸: 예상 비용.
pub const USAGE_COST: &str = "예상 비용";
/// 사용량 표 칸: 맥락 정리.
pub const USAGE_COMPACTIONS: &str = "맥락 정리";
/// 사용량 표 칸: 채점.
pub const USAGE_LABELS: &str = "채점";
/// judge 버전 화면 제목.
pub const JUDGE_VERSION_TITLE: &str = "판단기 버전";
/// judge 버전 표: 지금 쓰는 버전.
pub const JUDGE_VERSION_ACTIVE: &str = "사용 중";
/// judge 버전 상세 칸: 질문.
pub const JUDGE_VERSION_QUESTION: &str = "질문";
/// judge 버전 상세 칸: 목표 틀림 비율.
pub const JUDGE_VERSION_TARGET: &str = "목표 틀림 비율";
/// judge 버전 상세 칸: 기준값.
pub const JUDGE_VERSION_THRESHOLD: &str = "기준값";
/// judge 버전 상세 칸: 최근 200건 틀림.
pub const JUDGE_VERSION_RECENT: &str = "최근 200건 틀림";
/// judge 버전 상세 칸: 판단 수.
pub const JUDGE_VERSION_JUDGMENTS: &str = "판단 수";
/// judge 버전 사용 확인 한 줄.
pub const JUDGE_VERSION_CONFIRM: &str = "이 버전을 쓸까요? u 또는 Enter 확인 · Esc 취소";
/// judge 버전 화면 키 안내.
pub const JUDGE_VERSION_HINT: &str = "Enter 상세 · r 영점 복귀 · t 다시 학습 · u 사용 · Esc 닫기";
/// 학습 확인 창 제목.
pub const TRAIN_TITLE: &str = "판단 모델 학습";
/// 학습 확인 창 칸: 채점 후보.
pub const TRAIN_CANDIDATES: &str = "채점 후보";
/// 학습 확인 창 칸: 채점 모델.
pub const TRAIN_GRADER: &str = "채점 모델";
/// 학습 확인 창 칸: 예상 토큰.
pub const TRAIN_TOKENS: &str = "예상 토큰";
/// 학습 확인 창 칸: 기준값 조정 대상.
pub const TRAIN_TARGETS: &str = "기준값 조정 대상";
/// 학습 확인 창 칸: 모델 추가 학습.
pub const TRAIN_RETRAIN: &str = "모델 추가 학습";
/// 예.
pub const YES: &str = "예";
/// 아니오.
pub const NO: &str = "아니오";
/// 학습 확인 창 선택지: 취소.
pub const CANCEL: &str = "취소";

/// `?` 단축키 안내 표. 키와 한국어 설명. 설명은 `Lang::tr`로 바꿔 쓴다.
pub const SHORTCUTS: &[(&str, &str)] = &[
    ("Enter", "입력 제출"),
    ("Tab", "관계 판단 없이 대기"),
    ("Alt+Enter", "줄바꿈"),
    ("Alt+↑", "최근 입력 되돌리기"),
    ("Ctrl+C", "멈춤"),
    ("Ctrl+D", "종료"),
    ("Ctrl+G", "외부 에디터"),
    ("Ctrl+R", "입력 기록 검색"),
    ("Ctrl+T", "전체 기록"),
    ("!", "셸 명령"),
    ("/", "명령 목록"),
    ("@", "파일 목록"),
    ("$", "스킬 목록"),
];

/// 언어를 고를 때 보는 환경 변수. 앞에 있는 것이 먼저다.
const LOCALE_VARS: [&str; 3] = ["LC_ALL", "LC_MESSAGES", "LANG"];

/// 한국어 키와 영어 문구. 이 파일의 모든 문구 상수가 들어 있어야 한다(테스트로 확인).
const ENGLISH: &[(&str, &str)] = &[
    ("작업 중", "working"),
    ("생각 중", "thinking"),
    ("파일 읽는 중", "reading files"),
    ("파일 수정 중", "editing files"),
    ("명령 실행 중", "running command"),
    ("허가 기다림", "waiting for permission"),
    ("맥락 정리 중", "compacting context"),
    ("공급자 전환 중", "switching provider"),
    ("하위 에이전트", "subagents"),
    ("개 실행 중", "running"),
    ("Token -", "Token -"),
    ("Token", "Token"),
    ("판단 중", "judging"),
    ("학습", "training"),
    ("대기", "queued"),
    ("다음", "after"),
    ("판단 차례", "waiting for judge turn"),
    ("판단기 연결 기다림", "waiting for judge connection"),
    ("쓰기 차례", "waiting for write turn"),
    ("맥락 정리 뒤", "after compaction"),
    ("모든 작업 뒤", "after all tasks"),
    ("[보내기]", "[send]"),
    ("[취소]", "[cancel]"),
    ("보류", "held"),
    ("[이어서]", "[continue]"),
    ("멈춤", "stopped"),
    ("보류됨", "held"),
    ("/continue 로 이어서", "/continue to resume"),
    ("멈춤 확인 안 됨", "stop not confirmed"),
    ("개 남음", "remaining"),
    ("보류를 닫을까요?", "close this hold?"),
    ("보류를 닫았습니다", "hold closed"),
    ("자동 판단 일시 중단", "auto judgment paused"),
    (
        "새 입력 접수 중단 · 판단기 연결을 확인하세요",
        "new input paused · check the judge connection",
    ),
    ("바로 반영: 준비 중", "steer: not ready"),
    (
        "판단기 연결 없음 · 차례에 보냅니다",
        "judge unavailable · sending in order",
    ),
    ("다른 Saturn에서 실행 중", "running in another Saturn"),
    ("폴더 설정 오류", "folder settings error"),
    ("이전 설정 번호", "continuing with settings revision"),
    ("로 계속", ""),
    ("전달 중", "delivering"),
    ("반영됨", "applied"),
    ("실패", "failed"),
    ("결과 확인 필요", "result needs check"),
    ("맥락 정리 후 이어서 진행", "context compacted, continuing"),
    ("로 전환", "switched"),
    ("이번 요청", "this request"),
    ("판단기", "judge"),
    ("회", "calls"),
    ("에 이어서 보냈어요", "added to"),
    ("판단이 맞았나요? (선택)", "was this right? (optional)"),
    ("1 맞아요  2 아니에요  0 닫기", "1 yes  2 no  0 dismiss"),
    ("바로 새 작업으로 실행할까요?", "run as a new task now?"),
    ("[실행]", "[run]"),
    ("[그대로]", "[keep]"),
    ("채점할 판단", "judgments to grade"),
    ("쌓이면 실행할 수 있습니다", "needed to run"),
    ("붙여넣은 내용", "pasted"),
    ("자", "chars"),
    ("건", ""),
    ("/help 도움말 · Ctrl+C 멈춤", "/help help · Ctrl+C stop"),
    ("맥락", "context"),
    ("맥락 미확인", "context unknown"),
    ("모두 이어서", "continue all"),
    ("골라서 이어서", "choose what to continue"),
    ("그대로 두기", "leave as is"),
    ("실행", "run"),
    (
        "이 작업 동안 같은 명령 허용",
        "allow this command for this task",
    ),
    ("실행하지 않고 계속", "don't run, continue"),
    (
        "실행하지 않고 다르게 하라고 말하기",
        "don't run, tell it what to do instead",
    ),
    ("전체", "all"),
    ("확인 필요", "needs check"),
    ("실행 중", "running"),
    ("끝남", "done"),
    ("모델 미보고", "model not reported"),
    ("Saturn", "Saturn"),
    ("새 작업으로 보냈어요", "started as a new task"),
    ("대기열에 넣었어요", "queued"),
    ("종료 코드", "exit code"),
    ("알 수 없는 명령", "unknown command"),
    ("잘못된 인자", "invalid argument"),
    (
        "셸 명령을 실행하지 못했습니다",
        "failed to run shell command",
    ),
    ("개", ""),
    ("폴더", "folder"),
    ("확인 안 됨", "not checked"),
    ("판단기 키 입력", "judge key"),
    ("Enter 확인 · Esc 종료", "Enter confirm · Esc quit"),
    ("폴더 설정 신뢰", "trust folder settings"),
    ("경로", "path"),
    ("지문", "fingerprint"),
    ("적용되는 항목", "applied"),
    ("무시되는 항목", "ignored"),
    ("바뀐 줄", "changed lines"),
    ("적용하고 계속", "apply and continue"),
    ("적용하지 않고 계속", "continue without applying"),
    ("종료", "quit"),
    ("보류된 작업이 있습니다", "there are held tasks"),
    ("고른 작업 이어서", "continue selected"),
    ("이유", "reason"),
    (
        "허가를 기다리는 다른 작업",
        "other tasks waiting for permission:",
    ),
    ("단축키", "shortcuts"),
    ("작업 목록", "tasks"),
    ("작업 없음", "no tasks"),
    ("모델", "model"),
    ("하위 항목", "children"),
    ("검색", "search"),
    ("새 이름", "new name"),
    ("묶음", "group"),
    (
        "Enter 이동 · Esc 닫기 · Tab 필터 · c 이어서 · d 취소 · f 검색 · g 묶음 · n 새 채팅 · r 이름 · s 보내기",
        "Enter open · Esc close · Tab filter · c continue · d cancel · f search · g group · n new chat · r rename · s send",
    ),
    ("불러오는 중", "loading"),
    ("사용량", "usage"),
    ("대상", "who"),
    ("새 입력", "input"),
    ("캐시 읽기", "cache read"),
    ("캐시 쓰기", "cache write"),
    ("출력", "output"),
    ("추론", "reasoning"),
    ("판단기 호출", "judge calls"),
    ("예상 비용", "estimated cost"),
    ("채점", "labels"),
    ("맥락 정리", "compactions"),
    ("판단기 버전", "judge versions"),
    ("사용 중", "in use"),
    ("질문", "question"),
    ("목표 틀림 비율", "target error"),
    ("기준값", "threshold"),
    ("최근 200건 틀림", "errors in last 200"),
    ("판단 수", "judgments"),
    (
        "이 버전을 쓸까요? u 또는 Enter 확인 · Esc 취소",
        "use this version? u or Enter confirm · Esc cancel",
    ),
    (
        "Enter 상세 · r 영점 복귀 · t 다시 학습 · u 사용 · Esc 닫기",
        "Enter details · r reset · t retrain · u use · Esc close",
    ),
    ("판단 모델 학습", "train judge"),
    ("채점 후보", "candidates"),
    ("채점 모델", "grader"),
    ("예상 토큰", "estimated tokens"),
    ("기준값 조정 대상", "threshold targets"),
    ("모델 추가 학습", "retrain model"),
    ("예", "yes"),
    ("아니오", "no"),
    ("취소", "cancel"),
    ("입력 제출", "submit"),
    ("관계 판단 없이 대기", "queue without judging"),
    ("줄바꿈", "newline"),
    ("최근 입력 되돌리기", "recall latest input"),
    ("외부 에디터", "external editor"),
    ("입력 기록 검색", "search history"),
    ("전체 기록", "full transcript"),
    ("셸 명령", "shell command"),
    ("명령 목록", "commands"),
    ("파일 목록", "files"),
    ("스킬 목록", "skills"),
    ("도움말", "help"),
    ("대기 입력 지금 보내기", "send a queued input now"),
    ("보내기 전 입력 취소", "cancel an input before sending"),
    ("보류 이어서", "continue a hold"),
    ("판단 피드백", "judgment feedback"),
    ("판단 모델 버전", "judge versions"),
    ("판단 기록 켜기와 끄기", "turn judgment records on or off"),
];

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
    // cost: time O(1), heap O(1), stack O(1), io 3
    // basis: estimate
    /// 운영체제 언어로 고른다. 언어 환경 변수를 차례로 보고 처음 비어 있지 않은 값이
    /// `ko`로 시작하면 `Ko`, 그 밖(값 없음, `C`, `POSIX` 포함)은 `En`.
    /// 변수 순서 `LC_ALL` → `LC_MESSAGES` → `LANG`은 초안이다(docs/design/tui.md 초안 값).
    pub fn detect() -> Self {
        let value = LOCALE_VARS
            .iter()
            .filter_map(|name| std::env::var(name).ok())
            .find(|value| !value.is_empty());
        Self::from_locale(value.as_deref())
    }

    /// 언어 환경 변수 값 하나로 고른다. `ko`로 시작하면 `Ko`.
    pub fn from_locale(value: Option<&str>) -> Self {
        match value {
            Some(value) if value.to_ascii_lowercase().starts_with("ko") => Self::Ko,
            _ => Self::En,
        }
    }

    /// 한국어 키 문구를 이 언어로 바꾼다. `Ko`면 그대로, `En`이면 `english`에서 찾고 없으면 키 그대로(누락은 테스트로 막는다).
    pub fn tr(self, ko: &'static str) -> &'static str {
        match self {
            Self::Ko => ko,
            Self::En => english(ko).unwrap_or(ko),
        }
    }
}

// cost: time O(k), heap O(1), stack O(1)
// vars: k = ENGLISH.len()
// basis: estimate
/// 한국어 키의 영어 문구. 없으면 `None`.
fn english(ko: &str) -> Option<&'static str> {
    ENGLISH
        .iter()
        .find(|(key, _)| *key == ko)
        .map(|(_, en)| *en)
}

// cost: time O(d), heap O(d), stack O(1), alloc 2
// vars: d = 자릿수
// basis: estimate
/// 천 단위 쉼표 숫자(`3,210`, `1,204`). 두 언어 같다.
pub fn format_count(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// 경과 시간. `Ko`: 60초 미만 `45초`, 그 이상 `2분 31초`, 초가 0이면 `1분`. `En`: `45s`, `2m 31s`, `1m`. 초 미만은 버린다.
pub fn format_elapsed(lang: Lang, elapsed: Duration) -> String {
    let total = elapsed.as_secs();
    let (minutes, seconds) = (total / 60, total % 60);
    let (m, s) = match lang {
        Lang::Ko => ("분", "초"),
        Lang::En => ("m", "s"),
    };
    if minutes == 0 {
        format!("{seconds}{s}")
    } else if seconds == 0 {
        format!("{minutes}{m}")
    } else {
        format!("{minutes}{m} {seconds}{s}")
    }
}

/// 토큰 수를 천 단위 `K`로(`38K`, `200K`). 1,000 미만은 그대로, 나머지는 내림.
pub fn format_kilo(tokens: u64) -> String {
    if tokens < 1_000 {
        tokens.to_string()
    } else {
        format!("{}K", tokens / 1_000)
    }
}

/// provider 화면 이름. `Codex` → `codex`, `Claude` → `claude`. 두 언어 같다.
pub fn provider_name(provider: Provider) -> &'static str {
    match provider {
        Provider::Codex => "codex",
        Provider::Claude => "claude",
    }
}

/// 개수 `2개`, 영어 `2`.
pub fn format_items(lang: Lang, n: u64) -> String {
    format!("{}{}", format_count(n), lang.tr(ITEMS_SUFFIX))
}

#[cfg(test)]
mod tests {
    use super::*;

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn english_covers_every_phrase_constant() {
        let source = include_str!("i18n.rs");
        let phrases: Vec<&str> = source
            .lines()
            .filter_map(|line| line.strip_prefix("pub const "))
            .filter(|line| line.contains(": &str = \""))
            .filter_map(|line| line.split('"').nth(1))
            .collect();

        let missing: Vec<&&str> = phrases.iter().filter(|p| english(p).is_none()).collect();

        assert!(phrases.len() > 100, "phrase scan should find the constants");
        assert!(missing.is_empty(), "missing english: {missing:?}");
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn english_covers_shortcut_descriptions() {
        let missing: Vec<&str> = SHORTCUTS
            .iter()
            .map(|(_, desc)| *desc)
            .filter(|desc| english(desc).is_none())
            .collect();

        assert!(missing.is_empty(), "missing english: {missing:?}");
    }

    #[test]
    fn from_locale_korean_prefix_returns_ko() {
        assert_eq!(Lang::from_locale(Some("ko_KR.UTF-8")), Lang::Ko);
    }

    #[test]
    fn from_locale_other_or_missing_returns_en() {
        assert_eq!(Lang::from_locale(Some("C")), Lang::En);
        assert_eq!(Lang::from_locale(Some("en_US.UTF-8")), Lang::En);
        assert_eq!(Lang::from_locale(None), Lang::En);
    }

    #[test]
    fn tr_ko_returns_key_and_en_returns_translation() {
        assert_eq!(Lang::Ko.tr(WORKING), "작업 중");
        assert_eq!(Lang::En.tr(WORKING), "working");
    }

    #[test]
    fn format_count_inserts_thousands_separators() {
        assert_eq!(format_count(0), "0");
        assert_eq!(format_count(999), "999");
        assert_eq!(format_count(3_210), "3,210");
        assert_eq!(format_count(1_234_567), "1,234,567");
    }

    #[test]
    fn format_elapsed_uses_minutes_and_seconds() {
        assert_eq!(
            format_elapsed(Lang::Ko, Duration::from_millis(45_900)),
            "45초"
        );
        assert_eq!(
            format_elapsed(Lang::Ko, Duration::from_secs(151)),
            "2분 31초"
        );
        assert_eq!(format_elapsed(Lang::Ko, Duration::from_secs(60)), "1분");
        assert_eq!(format_elapsed(Lang::En, Duration::from_secs(151)), "2m 31s");
    }

    #[test]
    fn format_kilo_rounds_down() {
        assert_eq!(format_kilo(999), "999");
        assert_eq!(format_kilo(38_900), "38K");
        assert_eq!(format_kilo(200_000), "200K");
    }
}
