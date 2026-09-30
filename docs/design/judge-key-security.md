# judge 키 보호

| 항목 | 값 |
|---|---|
| 상태 | 결정 |
| 관련 결정 | [engine만 judge를 부르고 자식 프로세스 환경에서 judge 키를 지운다](../decisions/2026-09-29-engine-as-judge-proxy.md), [판단 규격은 Saturn이 정하고 judge는 중립 이름과 출처로 기록한다](../decisions/2026-09-29-vendor-neutral-judge-spec.md) |

## 요약

judge 키 보호는 외부 judge API 키를 provider와 subagent가 어떤 경로로도 받거나 읽지 못하게 하는 기능이다. 키는 정해진 네 방법으로만 받고, macOS 키체인에 OS API로 직접 저장한다. `engine`은 자식 프로세스 환경에서 키 변수를 지운다. Saturn 소유 PreToolUse 훅은 저장된 키를 찾아가는 도구 호출을 막는다. 키는 HTTPS로 허용 호스트에만 보내고, 로그와 오류와 기록에서는 가린다.

## 동기

기준 judge는 외부 API라 키가 필요하다. 키를 환경 변수로 주면 provider 자식 프로세스가 같은 환경을 그대로 받는다. provider와 subagent는 judge를 부를 일이 없으므로 키를 받을 이유도 없다. macOS `security` 명령으로 키체인에 저장하면 그 명령이 신뢰 앱이 된다(2026-09-29 확인). 그러면 다른 프로세스도 같은 명령으로 확인 창 없이 키를 읽을 수 있다(Silverfort 보고). Saturn은 provider 설정을 막지 않고 추적만 하지만, judge 키 보안만은 그 예외로 둔다.

## 예시

### 처음 실행 때 키를 입력할 때

1. 사용자가 저장된 키 없이 `saturn`을 실행한다.
2. `judges`의 시작 확인이 실패하고, TUI가 숨김 입력으로 judge 키를 요청한다.
3. 사용자가 키를 붙여 넣으면 `judges`가 `GET /v1/models`로 키를 확인하고 모델 버전을 고정한다.
4. `secrets`가 키를 macOS 키체인에 OS API로 직접 저장한다.
5. `settings`는 키의 출처와 끝 4자리만 설정에 적고, Saturn이 실행을 계속한다.

### CI에서 환경 변수로 키를 줄 때

1. 사용자가 파이프로 입력을 넘기는 CI에서 키 없이 `saturn`을 실행한다.
2. 입력할 수 없는 환경이므로 Saturn은 묻지 않고 끝내며 환경 변수와 표준 입력 방식을 안내한다.
3. 사용자가 환경 변수로 키를 주고 다시 실행한다.
4. engine이 provider를 실행할 때 자식 환경에서 제외 목록의 변수를 지운다.
5. provider와 그 아래 subagent는 키 변수가 없는 환경에서 작업한다.

### subagent가 저장된 키를 찾을 때

1. Claude subagent가 도구 호출로 키 저장소 조회 명령을 실행하려 한다.
2. engine이 실행별 설정으로 넘긴 Saturn 소유 PreToolUse 훅이 그 호출을 막는다.
3. 사용자가 따로 둔 훅은 그대로 동작한다.

## 상세 설계

### 키 입력

1. `secrets`가 숨김 입력, 표준 입력, 환경 변수, 비밀번호 관리자 명령 설정 중 하나로 키를 받는다.
2. 비밀번호 관리자 명령으로 받은 키는 메모리에만 둔다.
3. `judges`가 입력 직후 `GET /v1/models`로 키를 확인하고 모델 버전을 고정한다.
4. 확인에 성공하면 `secrets`가 키를 저장한다.

- judge 키는 명령 인자로 받지 않는다. 키 입력 방법을 위 네 가지로 한정하기 위해서다.
- 시작 확인이 실패하면 그 자리에서 키를 요청한다. 시작 확인 절차는 [judge](judge.md)에 있다.

### 키 저장

| 조건 | 저장 위치와 방식 |
|---|---|
| 기본 | macOS 키체인에 OS API로 직접 저장 |
| OS 저장소가 없음 | 권한 0600 파일에 저장 |
| 강화 방식 | 신뢰 앱 없는 키체인 항목으로 저장 |

- 키체인은 `keyring`으로 OS API를 직접 쓴다.
- 강화 방식이면 session 시작 때 키체인 암호를 한 번 요청한다.
- 강화 방식이면 비활성 10분이나 최대 12시간 뒤 키를 잠금 상태로 바꾼다.
- 설정에는 키의 출처와 끝 4자리만 적는다.
- judge 주소와 키 참조는 폴더 층에서 바꿀 수 없다. 사용자 전용 항목이기 때문이다.

### `security` 명령을 쓰지 않는 이유

키를 키체인에 저장할 때 `security` 명령을 쓰지 않는다. 그 명령으로 저장하면 명령 자체가 항목의 신뢰 앱이 된다. 그러면 누구든 같은 명령으로 확인 창 없이 키를 읽을 수 있다. OS API로 직접 저장한 항목을 확인 창 없이 읽는 경로가 있는지는 [#2](https://github.com/woonyong-choi/saturn/issues/2) 실험으로 확인한다.

### 키를 저장하지 않는 곳

- judge 키는 SQLite 기록 저장소, 로그, 영수증, 설정 파일에 저장하지 않는다. 저장된 곳을 에이전트가 찾아가 읽는 위험을 줄이기 위해서다.
- Authorization 헤더는 기록하지 않는다.

### 자식 환경의 변수 제거

1. engine이 `Supervisor`로 넘길 때 제외 목록의 변수를 지운다.
2. `Supervisor`가 provider로 넘길 때 제외 목록의 변수를 다시 지운다.

- 제외 목록은 `secrets` 모듈 한 곳에 두고 Saturn 내부 비밀 변수도 같은 목록으로 처리한다. 제거 대상을 한 곳에서 테스트로 고정하기 위해서다.
- 두 단계 모두에서 지운다. judge 키 환경 변수가 자식 프로세스에 그대로 전달되는 일을 막기 위해서다.

### Saturn 소유 PreToolUse 훅

engine은 Claude를 실행할 때 Saturn 소유 PreToolUse 훅을 실행별 설정으로 넘긴다.

- 훅은 키 저장소 조회 명령, 대체 파일 읽기, Saturn 비밀 파일 접근을 막는다. subagent가 저장된 키를 찾아가 읽는 일을 막기 위해서다.
- 사용자의 기존 훅은 감싸거나 지우지 않는다. provider 설정은 사용자에게 맡기기 때문이다.
- 훅이 키 저장소 접근을 실제로 막는지는 [#3](https://github.com/woonyong-choi/saturn/issues/3), subagent까지 적용되는지는 [#23](https://github.com/woonyong-choi/saturn/issues/23) 실험으로 확인한다.

### 전송

- judge는 `engine`만 부른다. judge 키가 자식 프로세스나 다른 호스트로 새는 일을 막기 위해서다.
- judge 전송은 HTTPS와 허용 호스트 `api.typesafe.ai`만 쓴다. 키가 다른 호스트로 가는 일을 막기 위해서다.
- 리다이렉트 때 인증 헤더를 지운다. 같은 이유다.
- TLS 검증을 끄는 설정은 두지 않는다. 같은 이유다.

### 출력 마스킹

- judge 키와 일치하는 문자열은 로그, 오류, 디버그 출력에서 가린다. 키가 provider 기록이나 TUI로 새는 일을 막기 위해서다.
- 판단 기록을 저장하기 전에 `secrets`가 보낸 원문과 받은 원문의 비밀값을 가린다.

### 오류 처리

| 상황 | 동작 |
|---|---|
| 키 입력 거부 | 원인 한 줄을 보이고 실행하지 않고 끝낸다. |
| 입력할 수 없는 환경(파이프, CI)의 judge 확인 실패 | 묻지 않고 끝내며 환경 변수와 표준 입력 방식을 안내한다. |

### 요구사항

| 요구사항 | 검증 계획 |
|---|---|
| provider 자식 프로세스 환경에는 judge 키 변수가 없다. | `secrets` 모듈 테스트로 자식 환경에 제외 목록의 이름이 없는지 확인한다. |
| judge 키는 기록 저장소, 로그, 오류 출력에 남지 않는다. | 키를 넣은 호출과 오류를 만든 뒤 저장소와 출력에 키 문자열이 없는지 확인한다. |
| 키체인에 직접 저장한 키는 확인 창 없이 읽히지 않는다. | [#2](https://github.com/woonyong-choi/saturn/issues/2) 실험으로 확인 창 없이 읽는 경로를 확인한다. |
| Saturn 소유 PreToolUse 훅은 키 저장소 접근을 막는다. | [#3](https://github.com/woonyong-choi/saturn/issues/3) 실험으로 훅의 차단을 확인한다. |
| 훅은 subagent의 도구 호출에도 적용된다. | [#23](https://github.com/woonyong-choi/saturn/issues/23) 실험으로 전경, 백그라운드, 중첩 subagent에 훅이 걸리는지 확인한다. |

## 단점

- 제외 목록과 훅 검사를 provider 변화에 맞춰 계속 유지한다.
- 강화 방식에서는 session을 시작할 때마다 키체인 암호를 입력한다.

## 대안

- 환경 변수로 키를 그대로 넘기는 방식은 provider와 subagent에 키가 드러나 버렸다([engine만 judge를 부르고 자식 프로세스 환경에서 judge 키를 지운다](../decisions/2026-09-29-engine-as-judge-proxy.md)).
- 암호문 파일에 저장하고 실행마다 암호를 확인하는 방식은 실행마다 암호를 입력해야 해 버렸다(같은 결정 기록).

## 미해결 질문

- judge 키 환경 변수와 키 명령 설정 키 이름을 `SATURN_JUDGE_KEY`, `judge.key_command`로 할지, 벤더 이름을 유지할지 ([#32](https://github.com/woonyong-choi/saturn/issues/32))
