//! 화면 문구 언어. 문구 키는 한국어 원문이고 `Lang::En`이면 번역표에서 찾는다.
//! 설계: docs/design/tui.md

use std::time::Duration;

use saturn_protocol::ids::Provider;

mod english;

use english::ENGLISH;

// 상태판 실행 줄
pub const WORKING: &str = "작업 중";
pub const THINKING: &str = "생각 중";
pub const READING_FILE: &str = "파일 읽는 중";
pub const EDITING_FILE: &str = "파일 수정 중";
pub const RUNNING_COMMAND: &str = "명령 실행 중";
pub const AWAITING_PERMISSION: &str = "허가 기다림";
pub const AWAITING_INPUT: &str = "입력 기다림";
pub const APPROVAL_PENDING: &str = "도구 사용 허가 준비 중";
/// `{minutes}`는 마지막 provider 이벤트 뒤 지난 분.
pub const NO_RESPONSE: &str = "응답 없음 {minutes}분";
pub const COMPACTING: &str = "맥락 정리 중";
pub const SWITCHING_PROVIDER: &str = "공급자 전환 중";
pub const SUBAGENTS: &str = "하위 에이전트";
pub const RUNNING_COUNT_SUFFIX: &str = "개 실행 중";
pub const TOKEN_UNREPORTED: &str = "Token -";
pub const TOKEN: &str = "Token";

// 상태판 판단 줄, 학습 줄
pub const JUDGING: &str = "판단 중";
pub const TRAINING: &str = "학습";

// 상태판 대기 줄
pub const QUEUED: &str = "대기";
pub const AFTER_TASK_SUFFIX: &str = "다음";
pub const ROUTER_ORDER: &str = "판단 차례";
pub const ROUTER_CONNECTION: &str = "라우터 연결 기다림";
pub const WRITE_TURN: &str = "쓰기 차례";
pub const AFTER_COMPACTION: &str = "맥락 정리 뒤";
pub const AFTER_ALL_TASKS: &str = "모든 작업 뒤";
pub const CONFIRM_STOP: &str = "멈출지 확인 중";
pub const BUTTON_SEND: &str = "[보내기]";
pub const BUTTON_CANCEL: &str = "[취소]";

// 상태판 보류 줄
pub const HELD: &str = "보류";
pub const BUTTON_CONTINUE: &str = "[이어서]";
pub const STOPPED: &str = "멈춤";
pub const HELD_DONE: &str = "보류됨";
pub const CONTINUE_HINT: &str = "/continue 로 이어서";
pub const STOP_UNCONFIRMED: &str = "멈춤 확인 안 됨";
pub const REMAINING_SUFFIX: &str = "개 남음";
pub const CLOSE_HELD_QUESTION: &str = "보류를 닫을까요?";
pub const HELD_CLOSED: &str = "보류를 닫았습니다";

// 상태판 알림 줄
pub const ROUTER_PAUSED: &str = "자동 판단 일시 중단";
pub const ROUTER_DISCONNECTED: &str = "판단 모델 연결 끊김";
pub const STEER_NOT_READY: &str = "바로 반영 준비 중";
pub const ROUTER_UNAVAILABLE_SEND: &str = "라우터 연결 없음 · 차례에 보냅니다";
pub const BUSY_ELSEWHERE: &str = "다른 Saturn에서 실행 중";
/// `{to}`는 이관한 스키마 버전.
pub const SCHEMA_MIGRATED: &str = "기록 저장소 v{to}로 옮김";
/// `{layer}`는 설정 층 이름, `{previous}`는 계속 쓰는 설정 번호, `{detail}`는 원인.
pub const SETTINGS_FALLBACK: &str = "{layer} 오류 · 이전 설정 번호 {previous}로 계속 · {detail}";
/// `{keys}`는 쉼표로 이은 키 이름.
pub const SETTINGS_IGNORED: &str = "폴더 설정의 무시한 항목 · {keys}";
/// `{line}`은 줄 번호, `{message}`는 검사기 원문.
pub const SETTINGS_PARSE_LINE: &str = "줄 {line}: {message}";
pub const SETTINGS_LAYER_DEFAULT: &str = "기본 설정";
pub const SETTINGS_LAYER_USER: &str = "사용자 설정";
pub const SETTINGS_LAYER_FOLDER: &str = "폴더 설정";
pub const SETTINGS_LAYER_CHAT: &str = "채팅 설정";
pub const SETTINGS_LAYER_RUN: &str = "실행 설정";

// 입력 전달 상태
pub const DELIVERING: &str = "전달 중";
pub const APPLIED: &str = "반영됨";
pub const REJECTED: &str = "거절됨";

// 대화 기록
pub const FAILED: &str = "실패";
pub const INTERRUPTED: &str = "중단됨";
pub const NEEDS_CHECK: &str = "결과 확인 필요";
pub const COMPACTED: &str = "맥락 정리 후 이어서 진행";
/// `{provider}`는 다시 시작한 provider 이름.
pub const PROVIDER_RESTARTED: &str = "{provider} 다시 시작함 · 변경된 권한 설정을 적용했습니다";
pub const PERMISSIONS_CHANGED: &str = "권한 설정 변경됨 · 다음 요청부터 적용됩니다";
pub const READ_ONLY_RUN_KEPT: &str =
    "읽기 전용으로 접수한 작업의 쓰기를 거부함 · 쓰려면 새 입력으로 보내세요";
/// `{provider}`는 끊긴 하위 에이전트를 다시 보낸 provider 이름.
pub const INTERRUPTED_SUBAGENT_RETURNED: &str = "{provider}가 크래시로 끊긴 하위 에이전트를 다시 시작해 작업을 멈췄습니다 · 이어 가려면 /continue";
pub const CONTEXT_DEFERRED: &str = "고정 제약이 길어 맥락 정리를 미룹니다";
pub const PACKET_OVERFLOW: &str = "맥락 한도 초과로 멈춤 · /continue로 다시 시도하세요";
pub const SWITCHED_SUFFIX: &str = "로 전환";
pub const REQUEST_SUMMARY: &str = "이번 요청";
pub const ROUTER_CALLS: &str = "라우터";
pub const TIMES_SUFFIX: &str = "회";
pub const FEEDBACK_STEERED: &str = "에 이어서 보냄";
pub const FEEDBACK_QUESTION: &str = "판단이 맞았나요? (선택)";
pub const FEEDBACK_CHOICES: &str = "1 맞음  2 틀림  0 닫기";
pub const CORRECTION_QUESTION: &str = "바로 새 작업으로 실행할까요?";
pub const BUTTON_RUN: &str = "[실행]";
pub const BUTTON_KEEP: &str = "[그대로]";
pub const TRAIN_SHORT: &str = "채점할 판단";
pub const TRAIN_SHORT_SUFFIX: &str = "쌓이면 실행할 수 있습니다";
pub const PASTED: &str = "붙여넣은 내용";
pub const CHARS_SUFFIX: &str = "자";
pub const COUNT_SUFFIX: &str = "건";

// 바닥줄
pub const FOOTER_HINT: &str = "/help 도움말 · Ctrl+C 멈춤";
pub const CONTEXT: &str = "맥락";
pub const CONTEXT_UNKNOWN: &str = "맥락 미확인";

// 창
pub const RESUME_ALL: &str = "모두 이어서";
pub const RESUME_PICK: &str = "골라서 이어서";
pub const RESUME_LEAVE: &str = "그대로 두기";
pub const TRAIN_RUN: &str = "실행";
pub const PERMISSION_ALLOW_ONCE: &str = "이번만 허용";
pub const PERMISSION_ALLOW_ALWAYS: &str = "항상 허용";
pub const PERMISSION_DENY: &str = "거부";
pub const FILTER_ALL: &str = "전체";
pub const FILTER_NEEDS_CHECK: &str = "확인 필요";
pub const FILTER_RUNNING: &str = "실행 중";
pub const FILTER_QUEUED: &str = "대기";
pub const FILTER_HELD: &str = "보류";
pub const FILTER_DONE: &str = "끝남";
pub const MODEL_UNREPORTED: &str = "모델 미보고";
pub const SOURCE_SATURN: &str = "Saturn";

// 초안: 아래 문구는 설계 표에 없다.
// 대화 기록 초안
pub const FEEDBACK_NEW_TASK: &str = "새 작업으로 보냄";
pub const FEEDBACK_QUEUED: &str = "대기열에 넣음";
pub const SHELL_EXIT: &str = "종료 코드";
pub const COMMAND_UNKNOWN: &str = "알 수 없는 명령";
pub const COMMAND_INVALID: &str = "잘못된 인자";
pub const SHELL_FAILED: &str = "셸 명령을 실행하지 못했습니다";
pub const ITEMS_SUFFIX: &str = "개";

// 시작 화면 초안
/// `ROUTER_CALLS`와 같은 키라 번역도 같다.
pub const START_ROUTER: &str = "라우터";
pub const START_FOLDER: &str = "폴더";
pub const START_ADDED_DIRS: &str = "더한 폴더";
pub const FOLDER_ADDED: &str = "폴더 더함";
pub const FOLDER_NEXT_SESSION: &str = "열린 session에는 다음 session부터 적용";
pub const VERSION_UNKNOWN: &str = "확인 안 됨";

// 창 초안
pub const ROUTER_KEY_TITLE: &str = "라우터 키 입력";
pub const ROUTER_KEY_HINT: &str = "Enter 확인 · Esc 종료";
pub const TRUST_TITLE: &str = "폴더 설정 신뢰";
pub const TRUST_PATH: &str = "경로";
pub const TRUST_FINGERPRINT: &str = "지문";
pub const TRUST_APPLIED: &str = "적용되는 항목";
pub const TRUST_IGNORED: &str = "무시되는 항목";
pub const TRUST_CHANGED: &str = "바뀐 줄";
pub const TRUST_APPLY: &str = "적용하고 계속";
pub const TRUST_SKIP: &str = "적용하지 않고 계속";
pub const TRUST_QUIT: &str = "종료";
/// `{count}`는 계속 처리될 작업 수.
pub const EXIT_TITLE: &str = "작업 {count}개 실행 중";
pub const EXIT_QUESTION: &str = "닫은 뒤 작업을 어떻게 할까요?";
pub const EXIT_CONTINUE: &str = "계속 실행";
pub const EXIT_STOP: &str = "멈추기";
pub const EXIT_HINT: &str = "↑↓ 이동 · Enter 선택 · Esc 닫지 않기";
/// TUI를 닫은 뒤 터미널에 남기는 한 줄. `{count}`는 계속 처리될 작업 수.
pub const EXIT_BACKGROUND: &str = "작업 {count}개 계속 실행 중 · saturn으로 다시 여세요";
pub const STOP_CONFIRM_TITLE: &str = "지금 멈추고 새 입력을 실행할까요?";
pub const STOP_CONFIRM_RUN: &str = "멈추고 실행";
pub const STOP_CONFIRM_HINT: &str = "↑↓ 이동 · Enter 선택 · Esc 대기";
pub const RESUME_TITLE: &str = "보류된 작업이 있습니다";
pub const RESUME_CONFIRM: &str = "고른 작업 이어서";
pub const PERMISSION_REASON: &str = "이유";
pub const PERMISSION_WAITING: &str = "허가를 기다리는 다른 작업";
pub const INPUT_REQUESTED: &str = "입력 요청";
pub const INPUT_LINK_NOTE: &str = "링크를 직접 열어 확인하세요 · Saturn은 열지 않습니다";
pub const INPUT_LINK_HELP: &str = "Enter 계속 · d 거절 · Esc 취소";
pub const INPUT_FORM_HELP: &str =
    "Tab 다음 칸 · ↑↓ 이동 · Space 선택 · Enter 확인 · Ctrl+D 거절 · Esc 취소";
pub const INPUT_WAITING: &str = "답을 기다리는 다른 요청";
pub const INPUT_REQUIRED: &str = "필수 항목입니다";
pub const INPUT_NOT_NUMBER: &str = "숫자를 입력하세요";
pub const INPUT_NOT_INTEGER: &str = "정수를 입력하세요";
pub const INPUT_YES: &str = "예";
pub const INPUT_NO: &str = "아니오";
pub const INPUT_OTHER: &str = "직접 입력";
pub const SHORTCUTS_TITLE: &str = "단축키";
pub const TASKS_TITLE: &str = "작업 목록";
pub const TASKS_EMPTY: &str = "작업 없음";
pub const TASKS_MODEL: &str = "모델";
pub const TASKS_CHILDREN: &str = "하위 항목";
pub const TASKS_SEARCH: &str = "검색";
pub const TASKS_RENAME: &str = "새 이름";
pub const TASKS_GROUP: &str = "묶음";
pub const TASKS_HELP: &str = "Enter 이동 · Esc 닫기 · Tab 필터 · a 폴더 범위 · c 이어서 · d 취소 · f 검색 · g 묶음 · n 새 채팅 · r 이름 · s 보내기";
pub const TASKS_SCOPE_CURRENT: &str = "현재 폴더";
pub const TASKS_SCOPE_ALL: &str = "모든 폴더";
pub const LOADING: &str = "불러오는 중";
pub const USAGE_TITLE: &str = "사용량";
pub const USAGE_WHO: &str = "대상";
pub const USAGE_INPUT: &str = "새 입력";
pub const USAGE_CACHE_READ: &str = "캐시 읽기";
pub const USAGE_CACHE_WRITE: &str = "캐시 쓰기";
pub const USAGE_OUTPUT: &str = "출력";
pub const USAGE_REASONING: &str = "추론";
pub const USAGE_ROUTER_CALLS: &str = "라우터 호출";
pub const USAGE_COST: &str = "예상 비용";
pub const USAGE_COMPACTIONS: &str = "맥락 정리";
pub const USAGE_LABELS: &str = "채점";
pub const USAGE_TOKENS: &str = "토큰";
pub const USAGE_TURNS: &str = "턴";
pub const USAGE_RANGE_CHAT: &str = "현재 채팅";
pub const USAGE_RANGE_DAY: &str = "최근 24시간";
pub const USAGE_RANGE_WEEK: &str = "최근 7일";
pub const USAGE_HELP: &str = "d 최근 24시간 · w 최근 7일 · Enter 자세히 · Esc 닫기";
pub const ROUTER_VERSION_TITLE: &str = "라우터 버전";
pub const ROUTER_VERSION_ACTIVE: &str = "사용 중";
pub const ROUTER_VERSION_QUESTION: &str = "질문";
pub const ROUTER_VERSION_TARGET: &str = "목표 틀림 비율";
pub const ROUTER_VERSION_THRESHOLD: &str = "기준값";
pub const ROUTER_VERSION_RECENT: &str = "최근 200건 틀림";
pub const ROUTER_VERSION_JUDGMENTS: &str = "판단 수";
pub const ROUTER_VERSION_CONFIRM: &str = "이 버전을 쓸까요? u 또는 Enter 확인 · Esc 취소";
pub const ROUTER_VERSION_HINT: &str = "Enter 상세 · r 영점 복귀 · t 다시 학습 · u 사용 · Esc 닫기";
pub const TRAIN_TITLE: &str = "판단 모델 학습";
pub const TRAIN_CANDIDATES: &str = "채점 후보";
pub const TRAIN_GRADER: &str = "채점 모델";
pub const TRAIN_TOKENS: &str = "예상 토큰";
pub const TRAIN_TARGETS: &str = "기준값 조정 대상";
pub const TRAIN_RETRAIN: &str = "모델 추가 학습";
pub const MODEL_TITLE: &str = "모델 고르기";
pub const MODEL_LOADING: &str = "모델 목록을 불러오는 중";
pub const MODEL_EMPTY: &str = "고를 수 있는 모델이 없습니다";
pub const MODEL_HINT: &str = "↑↓ 이동 · Enter 고르기 · Esc 취소";
/// `{provider}`와 `{model}` 자리는 호출하는 쪽이 채운다.
pub const MODEL_PINNED: &str = "다음 입력부터 {provider} · {model} 모델로 보냅니다";
pub const YES: &str = "예";
pub const NO: &str = "아니오";
pub const CANCEL: &str = "취소";

// CLI 출력. `{이름}` 자리는 호출하는 쪽이 채운다.
/// `{message}`는 engine이 준 원인.
pub const CLI_ROUTER_KEY_REQUIRED: &str = "router 키가 필요합니다({message}): SATURN_KEY 환경 변수나 router.key.command 설정을 정한 뒤 다시 실행하세요";
pub const CLI_CONFIRM_NEEDS_TERMINAL: &str = "확인을 받을 터미널이 없어 아무것도 바꾸지 않았습니다";
pub const CLI_EXPORTED: &str = "판단 기록을 내보냈습니다: {path}";
pub const CLI_PATH_UNRESOLVED: &str = "경로를 확인하지 못했습니다: {path}";
pub const CLI_PRUNED: &str = "기록을 정리했습니다";
pub const CLI_PRUNE_PREVIEW: &str =
    "아무것도 지우지 않았습니다 · 지울 기록 목록은 아직 볼 수 없으니 지우려면 --yes로 실행하세요";
pub const CLI_ROUTER_VERSION_NOT_FOUND: &str =
    "router 버전을 찾지 못했습니다: {version} (사용 가능: {available})";
pub const CLI_ROUTER_VERSION_ALREADY: &str = "이미 쓰는 router 버전입니다: {version}";
pub const CLI_ROUTER_VERSION_PROMPT: &str =
    "다음 router 버전을 쓸까요? {version} (현재: {current})";
pub const CLI_ROUTER_VERSION_NOT_CONFIRMED: &str =
    "확인하지 않았습니다 · router 버전은 {current} 그대로입니다";
pub const CLI_ROUTER_VERSION_NOW: &str = "router 버전을 바꿨습니다: {version}";
pub const CLI_NO_ROUTER_VERSIONS: &str = "engine이 router 버전 목록 없이 답했습니다";
pub const CLI_NO_USAGE_TABLE: &str = "engine이 사용량 표 없이 답했습니다";
pub const CLI_TRAIN_ACCEPTED: &str = "학습 요청을 받았습니다";
pub const CLI_TRAIN_PROMPT: &str = "학습을 실행할까요?";
/// `{stage}`는 engine이 준 단계 이름.
pub const CLI_TRAIN_PROGRESS: &str = "{stage} · 채점 {labeled}건 · {elapsed} · 토큰 {tokens}";
pub const CLI_TRAIN_FINISHED: &str = "학습을 마쳤습니다";
pub const CLI_TRAIN_CANCELLED: &str = "학습을 취소했습니다";
pub const CLI_NO_CHAT_TO_CONTINUE: &str =
    "이 폴더에 이어 열 채팅이 없습니다 · saturn으로 새 채팅을 시작하세요";
pub const CLI_NO_LATEST_CHAT_ANSWER: &str = "engine이 최근 채팅 없이 답했습니다";
pub const CLI_RESUME_UNSUPPORTED: &str = "채팅 id 없는 --resume과 --resume all은 아직 지원하지 않습니다: engine이 채팅 목록을 줄 수 없습니다 · --resume <chat id>를 쓰세요";
pub const CLI_ADD_DIR_UNREADABLE: &str = "--add-dir 폴더를 읽지 못했습니다: {dir}";
pub const CLI_ADD_DIR_NOT_FOLDER: &str = "--add-dir은 폴더여야 합니다: {dir}";
pub const CLI_CURRENT_DIR_UNREADABLE: &str = "현재 폴더를 읽지 못했습니다";
pub const CLI_NESTED: &str =
    "saturn은 에이전트 작업 안에서 실행할 수 없습니다({marker} 변수가 설정되어 있습니다)";
pub const CLI_ENGINE_NOT_FOUND: &str =
    "{binary} 실행 파일을 saturn 옆이나 PATH에서 찾지 못했습니다";
pub const CLI_ENGINE_START_FAILED: &str = "engine을 시작하지 못했습니다: {binary}";
pub const CLI_ENGINE_POLL_FAILED: &str = "engine 상태를 확인하지 못했습니다";
/// `{tail}`은 engine 로그의 끝 줄.
pub const CLI_ENGINE_EXITED: &str = "engine이 소켓을 열기 전에 끝났습니다: {socket}{tail}";
pub const CLI_ENGINE_TIMEOUT: &str =
    "engine이 {secs}초 안에 소켓을 열지 않았습니다: {socket}{tail}";
pub const CLI_LOG_PATH: &str = "(로그: {path})";
pub const CLI_ARGS_CONFLICT: &str =
    "--continue, --resume, --add-dir은 대화 화면을 여는 인자라 하위 명령과 함께 쓸 수 없습니다";
pub const CLI_RESUME_VALUE: &str = "채팅 id(숫자)나 `all`이어야 합니다: `{text}`";
pub const CLI_CONFIG_FORMAT: &str = "KEY=VALUE 형식이어야 합니다: `{text}`";
pub const CLI_CONFIG_EMPTY_KEY: &str = "키가 비어 있습니다: `{text}`";

/// (키, `Lang::tr`의 한국어 설명) 쌍.
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

/// 앞에 있는 것이 먼저다(초안).
const LOCALE_VARS: [&str; 3] = ["LC_ALL", "LC_MESSAGES", "LANG"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    Ko,
    #[default]
    En,
}

impl Lang {
    // cost: time O(1), heap O(1), stack O(1), io 3
    // basis: estimate
    /// 처음 비어 있지 않은 언어 환경 변수가 `ko`로 시작하면 `Ko`, 그 밖(`C`, `POSIX` 포함)은 `En`.
    pub fn detect() -> Self {
        let value = LOCALE_VARS
            .iter()
            .filter_map(|name| std::env::var(name).ok())
            .find(|value| !value.is_empty());
        Self::from_locale(value.as_deref())
    }

    pub fn from_locale(value: Option<&str>) -> Self {
        match value {
            Some(value) if value.to_ascii_lowercase().starts_with("ko") => Self::Ko,
            _ => Self::En,
        }
    }

    /// 번역이 없으면 키 그대로 돌려준다(누락은 테스트로 막는다).
    pub fn tr(self, ko: &str) -> &str {
        match self {
            Self::Ko => ko,
            Self::En => english(ko).unwrap_or(ko),
        }
    }
}

// cost: time O(k), heap O(1), stack O(1)
// vars: k = ENGLISH.len()
// basis: estimate
fn english(ko: &str) -> Option<&'static str> {
    ENGLISH
        .iter()
        .find(|(key, _)| *key == ko)
        .map(|(_, en)| *en)
}

// cost: time O(d), heap O(d), stack O(1), alloc 2
// vars: d = 자릿수
// basis: estimate
/// 천 단위 쉼표(`3,210`).
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

/// 초 미만은 버린다.
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

/// 1,000 미만은 그대로, 그 이상은 천 단위로 내림(`38K`).
pub fn format_kilo(tokens: u64) -> String {
    if tokens < 1_000 {
        tokens.to_string()
    } else {
        format!("{}K", tokens / 1_000)
    }
}

pub fn provider_name(provider: Provider) -> &'static str {
    match provider {
        Provider::Codex => "codex",
        Provider::Claude => "claude",
    }
}

/// 문장 안에서 쓰는 이름.
pub fn provider_title(provider: Provider) -> &'static str {
    match provider {
        Provider::Codex => "Codex",
        Provider::Claude => "Claude",
    }
}

pub fn format_items(lang: Lang, n: u64) -> String {
    format!("{}{}", format_count(n), lang.tr(ITEMS_SUFFIX))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 값이 다음 줄로 넘어간 상수도 읽는다.
    fn phrase_constants(source: &str) -> Vec<&str> {
        let mut lines = source.lines();
        let mut phrases = Vec::new();
        while let Some(line) = lines.next() {
            let Some(rest) = line.strip_prefix("pub const ") else {
                continue;
            };
            let Some((_, value)) = rest.split_once(": &str =") else {
                continue;
            };
            let value = if value.trim().is_empty() {
                lines.next().unwrap_or_default()
            } else {
                value
            };
            phrases.extend(value.split('"').nth(1));
        }
        phrases
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn english_covers_every_phrase_constant() {
        let source = include_str!("i18n.rs");
        let phrases = phrase_constants(source);

        let missing: Vec<&&str> = phrases.iter().filter(|p| english(p).is_none()).collect();

        assert!(phrases.len() > 100, "phrase scan should find the constants");
        assert!(missing.is_empty(), "missing english: {missing:?}");
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    // cost: time O(p), heap O(1), stack O(1)
    // vars: p = 문구 상수 수
    // basis: estimate
    #[test]
    fn korean_phrases_follow_claude_code_format() {
        const POLITE_ENDINGS: [&str; 6] = ["었어요", "았어요", "에요", "예요", "해요", "했어요"];
        let phrases = phrase_constants(include_str!("i18n.rs"));

        let broken: Vec<&&str> = phrases
            .iter()
            .filter(|phrase| {
                phrase.ends_with('.')
                    || phrase
                        .split(|c: char| !c.is_alphabetic())
                        .any(|word| POLITE_ENDINGS.iter().any(|end| word.ends_with(end)))
            })
            .collect();

        assert!(broken.is_empty(), "해요체 어미나 끝 마침표: {broken:?}");
    }

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
        assert_eq!(Lang::En.tr(WORKING), "Working");
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
