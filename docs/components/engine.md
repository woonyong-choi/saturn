# 엔진 설계

엔진은 사용자마다 하나씩 상주하며 핵심 규칙이 trait으로 정한 외부 연결을 구현한다. 아키텍처는 [아키텍처](../architecture.md)에 있다.

## 모듈

| 모듈 | 하는 일 | 위치 |
|---|---|---|
| `secrets` | `SecretStore`: 판단기 키 저장, 자식 환경 변수 제거, 출력의 비밀값 마스킹 | `saturn-terminal/engine/src/secrets/` |
| `store` | `Store`: SQLite 스키마, 자동 이관과 백업, 단일 writer 보장 | `saturn-terminal/engine/src/store/` |
| `settings` | `SettingsManager`: 설정 층 병합, 설정 스냅샷, 폴더 신뢰, 주석 보존 편집 | `saturn-terminal/engine/src/settings/` |
| `processes` | `Supervisor`: 공급자 프로세스 실행, 감시, 단계별 중지 | `saturn-terminal/engine/src/processes/` |
| `providers` | `CodexClient`, `ClaudeClient`: 공급자 연결, 명령과 스킬 전달, 공급자 용어 변환 | `saturn-terminal/engine/src/providers/` |
| `judges` | `RemoteJudge`(외부 판단기 API)와 `LocalJudge`(로컬 판단 모델) 연결 | `saturn-terminal/engine/src/judges/` |
| `rpc` | `RpcServer`: 화면과 Unix 소켓 위 JSON-RPC 통신, 여러 화면 동시 접속, 사용자당 엔진 하나 보장 | `saturn-terminal/engine/src/rpc/` |

## 동작

### 엔진 시작과 화면 접속

1. `rpc`: 사용자당 엔진 잠금 획득
2. `store`: 스키마 버전이 올라갔으면 스키마 이관
3. `settings`: 설정 층 병합과 설정 번호 확정
4. `judges`: 판단기 시작 확인
5. `rpc`: Unix 소켓에서 JSON-RPC 접속 수락
6. `rpc`: 여러 화면의 동시 접속 유지

### 스키마 이관

1. `store`: 새 버전을 처음 켤 때 스키마 버전이 올라갔으면 이관 직전 백업 생성
2. `store`: 이전 버전의 백업 삭제
3. `store`: 스키마 자동 이관
4. `store`: 이관 사실 한 줄 안내
5. `store`: 만든 지 14일이 지난 백업 자동 삭제

### 설정 적용

1. `settings`: 기본값 층 로드
2. `settings`: 사용자 층 `~/.saturn/config.toml` 로드
3. `settings`: 작업 폴더에서 git 맨 위까지 올라가며 폴더 층 `<작업 폴더>/.saturn/config.toml` 검색
4. `settings`: 처음 보거나 내용이 바뀐 폴더 설정이면 한 번 질문 뒤 경로와 지문으로 신뢰 기록
5. `settings`: 채팅 층과 이번 실행 `-c` 층 병합, 뒤 층 우선
6. `settings`: 병합 결과 검사
7. `store`: 검사를 통과하면 병합 결과와 층 목록을 새 설정 번호의 스냅샷으로 기록
8. `store`: 같은 내용의 스냅샷이 있으면 기존 설정 번호 재사용

### 판단기 시작 확인

1. `judges`: 판단 방식이 쓰는 판단기 선택
2. `judges`: 외부 판단기면 `GET /v1/models`와 실제 판단 1건으로 확인
3. `judges`: 로컬 판단 모델이면 모델 로드나 API 서버 응답으로 확인
4. `judges`: 확인에 실패하면 그 자리에서 숨김 입력으로 키 요청
5. `judges`: 받은 키로 다시 확인
6. `secrets`: 확인에 성공하면 키 저장 뒤 실행 계속

### 판단기 키 입력과 저장

1. `secrets`: 숨김 입력, 표준 입력, 환경 변수, 비밀번호 관리자 명령 설정 중 하나로 키 수신
2. `secrets`: 비밀번호 관리자 명령으로 받은 키는 메모리에만 보관
3. `judges`: 입력 직후 `GET /v1/models`로 키 확인과 모델 버전 고정
4. `secrets`: macOS 키체인에 OS API로 직접 저장
5. `secrets`: OS 저장소가 없으면 권한 0600 파일에 저장
6. `secrets`: 강화 방식이면 신뢰 앱 없는 항목으로 저장, 세션 시작 때 키체인 암호 한 번 요청
7. `secrets`: 강화 방식이면 비활성 10분이나 최대 12시간 뒤 키 잠금 상태 전환
8. `settings`: 키의 출처와 끝 4자리만 설정에 기록

### 공급자 실행

1. `settings`: 입력 접수 때 고정한 설정 번호의 값 조회
2. `secrets`: 자식 환경에서 제외 목록의 변수 제거
3. `providers`: 사용자 공급자 설정에 값이 없는 항목만 Saturn 기본값 인자 추가
4. `providers`: Claude 실행에 Saturn 소유 PreToolUse 훅을 실행별 설정으로 전달
5. `processes`: 공급자 프로세스 실행과 감시 시작
6. `providers`: Claude는 `system/init`의 `slash_commands`로 공급자 명령 목록 수집
7. `providers`: Codex는 명령 대응표와 `skills/list`로 공급자 명령 목록 수집

### 입력 전달

1. `providers`: 새 턴은 Codex `turn/start`, Claude 스트림 입력으로 전송
2. `providers`: 끼워 넣기는 Codex `turn/steer`, Claude 스트림 입력 추가로 전달
3. `providers`: 멈춤 신호는 Codex `turn/interrupt`, Claude interrupt 제어 요청으로 전달
4. `providers`: 맥락 정리는 Codex `thread/compact/start`, Claude `/compact` 전송으로 실행
5. `providers`: Claude 공급자 명령과 스킬은 프롬프트에 `/이름`을 그대로 넣어 전송
6. `providers`: Codex 공급자 명령은 명령 이름과 app-server 메서드 대응표로 호출

### 이벤트 수신

1. `providers`: 공급자 이벤트를 Saturn 용어의 이벤트로 변환
2. `providers`: Codex 신호를 공급자 세션 id(`thread_id`)별로 분리
3. `providers`: Codex 자식 작업을 부모 작업 아래 등록
4. `providers`: Claude 하위 에이전트를 Task/Agent 도구 호출 id와 `parent_tool_use_id`로 연결
5. `providers`: Codex `tokenUsage`는 세션 누적, Claude 사용량은 턴 값으로 범위 표시
6. `providers`: 공급자가 입력 없이 시작한 턴에 `origin = provider-wake` 표시
7. `store`: 이벤트, 하위 에이전트, 사용량 보고 원값 기록

### 세션 닫기와 재개

1. `providers`: Claude 세션 닫기는 프로그램 종료로 처리
2. `providers`: Claude 세션 재개는 stream-json 방식 `--resume`으로 처리
3. `providers`: Codex는 app-server 하나를 창구로 세션 정리와 세션 재개 처리
4. `store`: 닫은 세션의 공급자 세션 ID 보관
5. `store`: 밖에서 연 공급자 세션을 채팅에 연결, Saturn이 관리하는 세션은 연결 목록에서 제외

### 트리 전체 중지

1. `providers`: 추적된 하위부터 멈춤 신호 전송
2. `providers`: Codex는 자식 세션별 `turn/interrupt`, Claude는 멈춤 제어 신호 사용
3. `processes`: 10초 뒤 남은 프로세스가 있으면 공급자 프로세스 묶음에 중지 신호
4. `processes`: 그래도 남으면 강제 종료
5. `processes`: 트리 전체의 종료 확인 뒤 완료 보고

### 크래시 뒤 복구

1. `store`: 끝나지 않은 실행의 `effect_scope` 조회
2. `store`: `proven-by-config`, `proven-by-observation`이면 자동 재개 대상 분류
3. `providers`: 자동 재개 대상은 파일 상태 확인 뒤 새 입력으로 전송
4. `store`: `network-possible`, `unobserved`이면 보류 기록
5. `rpc`: 보류 항목마다 `/continue` 한 줄 제안

### 판단기 호출

1. `judges`: 요청을 `model`, `state`, `questions`로 구성
2. `judges`: `state`에서 비밀값, 절대 경로, 다른 대화 원문 제외
3. `judges`: 요청이 64K를 넘거나 `state`와 가장 긴 질문 합이 32K를 넘으면 나눠서 전송
4. `judges`: `choice` 선택지가 255개를 넘으면 계층 선택으로 분할
5. `judges`: 후보 밖 선택, NaN, 확률 누락이면 `invalid` 처리
6. `judges`: 판단 사이 채팅 revision이 바뀌었으면 `superseded` 처리
7. `secrets`: 저장 전 보낸 원문과 받은 원문의 비밀값 마스킹
8. `store`: 기록을 끈 채팅(`/record off`)이면 판단 기록 저장 생략
9. `store`: 그 밖의 채팅이면 모든 판단 호출의 원문, 답, 비용, 시간 기록

### 기록 보존과 삭제

1. `store`: 실행이 끝난 원시 기록을 gzip으로 압축 저장
2. `store`: 압축한 원시 기록은 읽을 때 자동 해제
3. `store`: 삭제 대상에서 열린 입력, 열린 실행, 처리 중인 중지 요청, 활성·대기 세션 제외
4. `store`: 삭제한 채팅의 ID, 해시, 삭제 시각을 삭제 흔적으로 기록
5. `store`: 삭제 뒤 `PRAGMA wal_checkpoint(TRUNCATE)`와 `VACUUM` 실행

### 화면 종료 뒤 동작

1. `rpc`: 화면이 끝나면 `on_exit` 값 확인
2. `providers`: `background`면 접수된 대기 입력을 순서대로 계속 전송
3. `rpc`: 허가 요청은 사용자 확인 대기로 유지, 화면이 다시 붙으면 먼저 전달
4. `rpc`: 화면 없는 동안 피드백 질문 생략
5. `rpc`: 완료 알림(macOS)을 켠 경우만 알림 전송
6. `processes`: 트리 전체가 끝나면 5분 유예 뒤 세션 닫기와 엔진 종료

## 규칙

| 규칙 | 이유 |
|---|---|
| 공급자 고유 이름은 `providers/codex`, `providers/claude` 안에서만 쓴다. | 화면과 앱이 공급자를 몰라도 그릴 수 있게 하기 위해서다. |
| 공급자마다 켜 둔 채 입력을 받는 연결을 둔다. | 한 번 실행 방식으로는 끼워 넣기가 불가능하기 때문이다. |
| 끼워 넣기 실측을 통과하기 전의 공급자는 끼워 넣기를 대기로 바꿔 처리한다. | 끼워 넣기 경로가 문서대로 동작하는지 실측으로 확인해야 하기 때문이다. |
| 그 공급자에 끼워 넣기를 대기로 바꿀 때 화면에 `바로 반영: 준비 중`을 보인다. | 사용자가 바로 반영되지 않는 이유를 알게 하기 위해서다. |
| 공급자의 샌드박스, 네트워크, 권한, 하위 에이전트 설정을 막거나 바꾸지 않는다. | 공급자 설정은 사용자에게 맡기고 Saturn은 추적만 하기 때문이다. |
| Saturn 기본값은 사용자 공급자 설정에 값이 없을 때만 실행 인자로 넘긴다. | 사용자가 정한 공급자 설정을 덮어쓰지 않기 위해서다. |
| Saturn 기본값은 권한(수정 허용)과 자동 압축 안전망 값이다. | 사용자 설정이 없을 때도 동작을 정해 두기 위해서다. |
| 안전망 인자는 Claude `--autocompact`, Codex `-c model_auto_compact_token_limit`다. | 사용자 설정 파일을 건드리지 않고 실행 인자로만 넘기기 위해서다. |
| 공급자 설정을 바꾸는 공급자 명령은 막지 않고 실제 적용된 값을 읽어 기록한다. | 공급자 설정을 막지 않으면서 추적은 하기 위해서다. |
| 공급자 명령 목록에서 화면 전용 명령과 Saturn 세션 명령이 대신하는 명령은 뺀다. | 뒤에서 연결할 때 쓸 수 없거나 Saturn 세션 기록과 어긋나기 때문이다. |
| Codex 작업 끝은 부모 작업의 `turn/completed`로만 판정한다. | 자식 작업의 끝을 작업 끝으로 잘못 보지 않기 위해서다. |
| 사용량 보고는 원값과 범위를 그대로 넘기고 0으로 채우지 않는다. | Codex 세션 누적을 그대로 더하면 중복 계산되기 때문이다. |
| 실행마다 `effect_scope`를 기록하고 크래시 뒤에는 이 값으로만 나눈다. | 전달 여부가 불확실한 입력을 자동으로 다시 보내지 않기 위해서다. |
| `proven-by-config`는 적용 설정을 읽어 트리 전체의 외부 효과가 불가능함을 증명한 값이다. | 설정으로 증명한 실행만 자동으로 이어 가기 위해서다. |
| `proven-by-observation`은 트리 전체를 끊김 없이 관찰했고 행동이 모두 로컬 전용인 값이다. | 관찰로 증명한 실행만 자동으로 이어 가기 위해서다. |
| 완료 신호 없는 흐름 끝, 읽는 중 종료, 끝이 없는 하위는 관찰 끊김으로 보고 `unobserved`를 쓴다. | 관찰하지 못한 외부 효과가 있을 수 있기 때문이다. |
| 자동으로 이어 갈 때 같은 패킷을 다시 보내지 않는다. | 이미 반영된 입력을 두 번 실행하지 않기 위해서다. |
| 멈춤은 트리 전체가 종료된 것을 확인하기 전에는 완료라고 하지 않는다. | 하위 에이전트가 남은 채 멈췄다고 보이는 일을 막기 위해서다. |
| 판단기 모델은 버전을 고정하고, 별칭을 쓰면 응답의 `model`을 기록한다. | 판단을 재현하기 위해서다. |
| 판단기 호출 한도와 지출 상한을 두지 않고 호출 수와 예상 비용만 보인다. | 계정 상한 정보가 없어 설계 근거가 없기 때문이다. |
| 판단기 키는 명령 인자로 받지 않는다. | 키 입력 방법을 숨김 입력, 표준 입력, 환경 변수, 비밀번호 관리자 명령으로 한정하기 위해서다. |
| 키를 키체인에 저장할 때 `security` 명령을 쓰지 않는다. | 그 명령이 신뢰 앱이 되어 누구나 조용히 읽을 수 있기 때문이다. |
| 판단기 키는 SQLite, 로그, 영수증, 설정 파일에 저장하지 않는다. | 저장된 곳을 에이전트가 찾아가 읽는 위험을 줄이기 위해서다. |
| 엔진에서 `Supervisor`로, `Supervisor`에서 공급자로 넘길 때 모두 제외 목록의 변수를 지운다. | 판단기 키 환경 변수가 자식 프로세스에 그대로 전달되는 일을 막기 위해서다. |
| 제외 목록은 `secrets` 모듈 한 곳에 두고 Saturn 내부 비밀 변수도 같은 목록으로 처리한다. | 제거 대상을 한 곳에서 테스트로 고정하기 위해서다. |
| Saturn 소유 PreToolUse 훅은 키 저장소 조회 명령, 대체 파일 읽기, Saturn 비밀 파일 접근을 막는다. | 하위 에이전트가 저장된 키를 찾아가 읽는 일을 막기 위해서다. |
| 사용자의 기존 훅은 감싸거나 지우지 않는다. | 공급자 설정은 사용자에게 맡기기 때문이다. |
| 판단기 전송은 HTTPS와 허용 호스트 `api.typesafe.ai`만 쓴다. | 키가 다른 호스트로 가는 일을 막기 위해서다. |
| 리다이렉트 때 인증 헤더를 지우고, TLS 검증을 끄는 설정을 두지 않는다. | 키가 다른 호스트로 가는 일을 막기 위해서다. |
| 설정 원본은 파일이고 기록 저장소에는 적용된 설정의 스냅샷만 둔다. | 입력마다 그때 쓴 설정을 번호로 남기기 위해서다. |
| 입력은 접수 때 고정한 설정 번호로 끝까지 처리한다. | 처리 중 설정이 바뀌어도 한 입력이 한 설정으로 처리되게 하기 위해서다. |
| 같은 내용의 설정은 기존 설정 번호를 다시 쓴다. | 여러 프로세스가 같은 설정에 번호를 중복으로 만드는 일을 막기 위해서다. |
| 판단기 주소와 키 참조, 채점 모델, 데이터 공유 동의, 판단 방식은 폴더 층에서 바꿀 수 없다. | 이 항목들은 사용자 전용이기 때문이다. |
| 잠깐 들여다본 다른 폴더의 설정은 적용하지 않고 참고 자료로만 읽는다. | 작업 폴더가 아닌 폴더의 설정이 실행에 섞이는 일을 막기 위해서다. |
| 서브 채팅은 부모 채팅 층을 물려받는다. | 서브가 메인과 같은 설정으로 실행되게 하기 위해서다. |
| 설정 파일은 읽은 버전을 확인한 뒤 쓰고 주석을 보존한다. | 다른 곳에서 고친 내용과 사용자 주석을 잃지 않기 위해서다. |
| 이관 직전 백업은 가장 최근 1개만 두고 14일 뒤 지운다. | 이관 규칙 버그에 대비한 임시본이기 때문이다. |
| 기록은 기본으로 무제한 보존하고 자동 정리는 설정으로 켤 때만 시작 때 한 번 실행한다. | 기록을 재현과 판단 모델 학습에 쓰기 때문이다. |
| 실행 중인 원시 기록은 압축하지 않고, 원본 해시와 크기는 압축 전 값으로 기록한다. | 압축이 기록 내용과 대조 값을 바꾸지 않게 하기 위해서다. |
| 채팅을 지워도 ID, 해시, 삭제 시각을 삭제 흔적으로 남긴다. | 같은 ID가 다시 들어오면 알아보기 위해서다. |
| 삭제는 `secure_delete=ON`으로 지운 자리를 덮어쓴다. | 지운 내용이 파일에 남는 일을 막기 위해서다. |
| 판단 기록은 일반 정리에서 제외하고 판단 기록 전용 정리로만 지운다. | 판단 기록을 판단 모델 학습에 쓰기 때문이다. |
| `~/.claude`, `~/.codex`의 공급자 기록은 지우지 않는다. | Saturn 소유가 아니기 때문이다. |
| `on_exit` 값은 `background`, `stop`, `ask`이고 기본은 `background`다. | 화면을 꺼도 작업을 계속할 수 있게 하기 위해서다. |
| 화면이 없는 동안 보류는 그대로 둔다. | 사용자가 멈춘 작업을 자동으로 이어 가지 않기 위해서다. |
| 에이전트가 작업 중 실행한 `saturn`은 부모 Saturn과 연결하는 기능이 생기기 전까지 거절한다. | 부모와 자식의 연결 규칙 없이 에이전트가 Saturn을 다시 실행하는 일을 막기 위해서다. |
| 설치 검증 테스트 전용으로 판단기 시작 확인을 건너뛰는 설정을 두고 도움말에 보이지 않는다. | 판단기 없이 설치만 검증하기 위해서다. |

## 오류 처리

| 상황 | 동작 |
|---|---|
| Codex `turn/steer`가 활성 턴 없음으로 실패 | 확정 미전달로 기록하고 다시 판단하지 않고 같은 세션에 `turn/start`로 보낸다. |
| Claude 끼워 넣기 중 턴 종료 | 공급자가 추가 메시지를 다음 턴에 처리하므로 따로 처리하지 않는다. |
| 멈춤 뒤 묶음 밖으로 빠져나간 프로세스 존재 | 완료라고 하지 않고 `멈춤 확인 안 됨 · N개 남음`을 보고한다. |
| 엔진 비정상 종료 | 다시 시작한 엔진이 `effect_scope`로 자동 이어 가기와 보류를 나눈다. |
| 공급자 흐름의 관찰 중단 | `effect_scope`를 `unobserved`로 기록하고 자동으로 이어 가지 않는다. |
| 판단기 시작 확인 실패나 키 입력 거부 | 원인 한 줄을 보이고 실행하지 않고 끝낸다. |
| 입력할 수 없는 환경(파이프, CI)의 판단기 확인 실패 | 묻지 않고 끝내며 환경 변수와 표준 입력 방식을 안내한다. |
| 판단기 호출 연속 3회 실패 | 새 입력 접수를 멈추고 연결 복구를 안내한다. |
| 판단기 요청 전송 전 실패 | 설정된 횟수 안에서 다시 보낸다. |
| 판단기 요청 전송 뒤 타임아웃 | `cost-unknown`으로 기록하고 다시 보내지 않는다. |
| 판단기 속도 제한 | 기다렸다가 다시 보낸다. |
| 설정 검사 실패 | 이전 설정 번호를 유지하고 경고한다. |
| 시작 때 이전 설정 번호 없음 | 실행하지 않는다. |
| 신뢰한 폴더 설정의 내용 변경 | 다시 묻고 새 지문으로 신뢰를 기록한다. |
| 열린 항목을 포함한 삭제 요청 | 열린 항목은 지우지 않고, 먼저 해당 작업을 끝내거나 멈추게 한다. |

## 검증

| 대상 | 확인 방법 |
|---|---|
| 자식 환경의 제외 목록 변수 제거 | `secrets` 모듈 테스트: 자식 환경에 제외 목록 이름 없음 확인 |
| 키체인 직접 저장 항목의 조용한 읽기 차단 | [#2 키체인 직접 읽기](https://github.com/woonyong-choi/saturn/issues/2) |
| Claude PreToolUse 훅의 키 저장소 접근 차단 | [#3 훅 키 차단](https://github.com/woonyong-choi/saturn/issues/3) |
| 하위 에이전트까지 훅 적용, 중첩 `saturn` 실행 거절 | [#23 하위 에이전트 훅](https://github.com/woonyong-choi/saturn/issues/23) |
| `effect_scope` 설정 증명의 재료 | [#4 공급자 적용 설정 보고](https://github.com/woonyong-choi/saturn/issues/4) |
| `effect_scope` 증명이 놓치는 외부 효과 경로 | [#22 하위 에이전트 외부 효과](https://github.com/woonyong-choi/saturn/issues/22) |
| 끼워 넣기와 멈춤 신호 전달 경로 | [#5 끼워 넣기 경로](https://github.com/woonyong-choi/saturn/issues/5) |
| Codex 끼워 넣기 불가 상태 목록 | [#27 Codex 끼워 넣기 불가 상태](https://github.com/woonyong-choi/saturn/issues/27) |
| 세션 닫기 뒤 재개 시간, 캐시 적중, Codex 재개 방법 | [#10 세션 재개 캐시](https://github.com/woonyong-choi/saturn/issues/10) |
| Claude 하위 에이전트 이벤트 수신 | [#17 Claude 하위 에이전트 이벤트](https://github.com/woonyong-choi/saturn/issues/17) |
| Claude 백그라운드 하위 에이전트 중지 | [#18 Claude 백그라운드 정지](https://github.com/woonyong-choi/saturn/issues/18) |
| Claude 사용량 보고 범위 | [#19 Claude 사용량 범위](https://github.com/woonyong-choi/saturn/issues/19) |
| Codex 자식 세션 추적과 중지 | [#20 Codex 자식 세션](https://github.com/woonyong-choi/saturn/issues/20) |
| 강제 종료 뒤 하위 에이전트 재개 | [#24 강제 종료 뒤 재개](https://github.com/woonyong-choi/saturn/issues/24) |
| Claude 스트림의 공급자 명령 결과와 허가 요청 | [#26 Claude 스트림 슬래시 명령](https://github.com/woonyong-choi/saturn/issues/26) |
