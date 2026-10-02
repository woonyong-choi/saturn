//! 화면 문구 언어. 문구 키는 한국어 원문이고 `Lang::En`이면 번역표에서 찾는다.
//! 설계: docs/design/tui.md

use std::time::Duration;

use saturn_protocol::ids::Provider;

// 상태판 실행 줄
pub const WORKING: &str = "작업 중";
pub const THINKING: &str = "생각 중";
pub const READING_FILE: &str = "파일 읽는 중";
pub const EDITING_FILE: &str = "파일 수정 중";
pub const RUNNING_COMMAND: &str = "명령 실행 중";
pub const AWAITING_PERMISSION: &str = "허가 기다림";
pub const APPROVAL_PENDING: &str = "도구 사용 허가 준비 중";
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
pub const JUDGE_ORDER: &str = "판단 차례";
pub const JUDGE_CONNECTION: &str = "판단기 연결 기다림";
pub const WRITE_TURN: &str = "쓰기 차례";
pub const AFTER_COMPACTION: &str = "맥락 정리 뒤";
pub const AFTER_ALL_TASKS: &str = "모든 작업 뒤";
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
pub const JUDGE_PAUSED: &str = "자동 판단 일시 중단";
pub const JUDGE_DISCONNECTED: &str = "판단 모델 연결 끊김";
pub const STEER_NOT_READY: &str = "바로 반영: 준비 중";
pub const JUDGE_UNAVAILABLE_SEND: &str = "판단기 연결 없음 · 차례에 보냅니다";
pub const BUSY_ELSEWHERE: &str = "다른 Saturn에서 실행 중";
/// `{to}`는 이관한 스키마 버전.
pub const SCHEMA_MIGRATED: &str = "기록 저장소 v{to}로 옮김";
pub const SETTINGS_ERROR: &str = "폴더 설정 오류";
pub const SETTINGS_PREVIOUS: &str = "이전 설정 번호";
pub const SETTINGS_CONTINUE_SUFFIX: &str = "로 계속";

// 입력 전달 상태
pub const DELIVERING: &str = "전달 중";
pub const APPLIED: &str = "반영됨";

// 대화 기록
pub const FAILED: &str = "실패";
pub const NEEDS_CHECK: &str = "결과 확인 필요";
pub const COMPACTED: &str = "맥락 정리 후 이어서 진행";
pub const CONTEXT_DEFERRED: &str = "고정 제약이 길어 맥락 정리를 미룹니다";
pub const SWITCHED_SUFFIX: &str = "로 전환";
pub const REQUEST_SUMMARY: &str = "이번 요청";
pub const JUDGE_CALLS: &str = "판단기";
pub const TIMES_SUFFIX: &str = "회";
pub const FEEDBACK_STEERED: &str = "에 이어서 보냈어요";
pub const FEEDBACK_QUESTION: &str = "판단이 맞았나요? (선택)";
pub const FEEDBACK_CHOICES: &str = "1 맞아요  2 아니에요  0 닫기";
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
pub const FEEDBACK_NEW_TASK: &str = "새 작업으로 보냈어요";
pub const FEEDBACK_QUEUED: &str = "대기열에 넣었어요";
pub const SHELL_EXIT: &str = "종료 코드";
pub const COMMAND_UNKNOWN: &str = "알 수 없는 명령";
pub const COMMAND_INVALID: &str = "잘못된 인자";
pub const SHELL_FAILED: &str = "셸 명령을 실행하지 못했습니다";
pub const ITEMS_SUFFIX: &str = "개";

// 시작 화면 초안
/// `JUDGE_CALLS`와 같은 키라 번역도 같다.
pub const START_JUDGE: &str = "판단기";
pub const START_FOLDER: &str = "폴더";
pub const START_ADDED_DIRS: &str = "더한 폴더";
pub const FOLDER_ADDED: &str = "폴더 더함";
pub const FOLDER_NEXT_SESSION: &str = "열린 session에는 다음 session부터 적용";
pub const VERSION_UNKNOWN: &str = "확인 안 됨";

// 창 초안
pub const JUDGE_KEY_TITLE: &str = "판단기 키 입력";
pub const JUDGE_KEY_HINT: &str = "Enter 확인 · Esc 종료";
pub const TRUST_TITLE: &str = "폴더 설정 신뢰";
pub const TRUST_PATH: &str = "경로";
pub const TRUST_FINGERPRINT: &str = "지문";
pub const TRUST_APPLIED: &str = "적용되는 항목";
pub const TRUST_IGNORED: &str = "무시되는 항목";
pub const TRUST_CHANGED: &str = "바뀐 줄";
pub const TRUST_APPLY: &str = "적용하고 계속";
pub const TRUST_SKIP: &str = "적용하지 않고 계속";
pub const TRUST_QUIT: &str = "종료";
pub const RESUME_TITLE: &str = "보류된 작업이 있습니다";
pub const RESUME_CONFIRM: &str = "고른 작업 이어서";
pub const PERMISSION_REASON: &str = "이유";
pub const PERMISSION_WAITING: &str = "허가를 기다리는 다른 작업";
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
pub const USAGE_JUDGE_CALLS: &str = "판단기 호출";
pub const USAGE_COST: &str = "예상 비용";
pub const USAGE_COMPACTIONS: &str = "맥락 정리";
pub const USAGE_LABELS: &str = "채점";
pub const USAGE_TOKENS: &str = "토큰";
pub const USAGE_TURNS: &str = "턴";
pub const JUDGE_VERSION_TITLE: &str = "판단기 버전";
pub const JUDGE_VERSION_ACTIVE: &str = "사용 중";
pub const JUDGE_VERSION_QUESTION: &str = "질문";
pub const JUDGE_VERSION_TARGET: &str = "목표 틀림 비율";
pub const JUDGE_VERSION_THRESHOLD: &str = "기준값";
pub const JUDGE_VERSION_RECENT: &str = "최근 200건 틀림";
pub const JUDGE_VERSION_JUDGMENTS: &str = "판단 수";
pub const JUDGE_VERSION_CONFIRM: &str = "이 버전을 쓸까요? u 또는 Enter 확인 · Esc 취소";
pub const JUDGE_VERSION_HINT: &str = "Enter 상세 · r 영점 복귀 · t 다시 학습 · u 사용 · Esc 닫기";
pub const TRAIN_TITLE: &str = "판단 모델 학습";
pub const TRAIN_CANDIDATES: &str = "채점 후보";
pub const TRAIN_GRADER: &str = "채점 모델";
pub const TRAIN_TOKENS: &str = "예상 토큰";
pub const TRAIN_TARGETS: &str = "기준값 조정 대상";
pub const TRAIN_RETRAIN: &str = "모델 추가 학습";
pub const YES: &str = "예";
pub const NO: &str = "아니오";
pub const CANCEL: &str = "취소";

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

/// 이 파일의 모든 문구 상수가 들어 있어야 한다(테스트로 확인).
const ENGLISH: &[(&str, &str)] = &[
    ("작업 중", "working"),
    ("생각 중", "thinking"),
    ("파일 읽는 중", "reading files"),
    ("파일 수정 중", "editing files"),
    ("명령 실행 중", "running command"),
    ("허가 기다림", "waiting for permission"),
    ("도구 사용 허가 준비 중", "preparing tool permission"),
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
    ("판단 모델 연결 끊김", "judge model disconnected"),
    ("바로 반영: 준비 중", "steer: not ready"),
    (
        "판단기 연결 없음 · 차례에 보냅니다",
        "judge unavailable · sending in order",
    ),
    ("다른 Saturn에서 실행 중", "running in another Saturn"),
    ("기록 저장소 v{to}로 옮김", "record store migrated to v{to}"),
    ("폴더 설정 오류", "folder settings error"),
    ("이전 설정 번호", "continuing with settings revision"),
    ("로 계속", ""),
    ("전달 중", "delivering"),
    ("반영됨", "applied"),
    ("실패", "failed"),
    ("결과 확인 필요", "result needs check"),
    ("맥락 정리 후 이어서 진행", "context compacted, continuing"),
    (
        "고정 제약이 길어 맥락 정리를 미룹니다",
        "pinned constraints are long, deferring context compaction",
    ),
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
    ("이번만 허용", "allow once"),
    ("항상 허용", "always allow"),
    ("거부", "deny"),
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
    ("더한 폴더", "added folders"),
    ("폴더 더함", "folder added"),
    (
        "열린 session에는 다음 session부터 적용",
        "applies to open sessions from the next session",
    ),
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
        "Enter 이동 · Esc 닫기 · Tab 필터 · a 폴더 범위 · c 이어서 · d 취소 · f 검색 · g 묶음 · n 새 채팅 · r 이름 · s 보내기",
        "Enter open · Esc close · Tab filter · a folder scope · c continue · d cancel · f search · g group · n new chat · r rename · s send",
    ),
    ("현재 폴더", "this folder"),
    ("모든 폴더", "all folders"),
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
    ("토큰", "tokens"),
    ("턴", "turns"),
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
    ("권한 모드 바꾸기", "change the permission mode"),
    ("폴더 더하기", "add a folder to the chat"),
];

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
