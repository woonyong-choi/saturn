# Codex 실행 중 설정 다시 읽기: 실험 설계

| 항목 | 값 |
|---|---|
| 이슈 | [#289](https://github.com/woonyong-choi/saturn/issues/289), [#284](https://github.com/woonyong-choi/saturn/issues/284) |
| 관련 설계 | [권한](../../design/permissions.md), [입력 요청](../../design/input-requests.md) |
| 사전 데이터 | 없음. 기존 실험의 설계·driver와 Saturn의 Codex home 생성 코드만 읽고 provider 실행 결과는 수집 전에 열람하지 않는다 |

## 질문

Codex app-server가 실행 중 `experimentalFeature/enablement/set`으로 `default_mode_request_user_input`을 즉시 바꾸어 같은 thread의 다음 입력에 반영하는지 확인한다. Codex가 시작할 때 읽은 `rules/default.rules`의 execpolicy를 파일 변경 또는 공식 설정 재읽기 요청으로 다시 읽는지, 같은 process와 새 process의 `touch` 행동으로 구별한다.

## 가설

| 가설 | 예측 | 적용 범위 |
|---|---|---|
| H1 | 질문 기능은 같은 process·thread에서 `켜짐 → 꺼짐 → 켜짐`으로 전환되며, 각 상태의 유도 입력은 각각 `item/tool/requestUserInput` 요청을 `옴 → 오지 않음 → 옴` 순서로 만든다. | Codex app-server, 전용 `CODEX_HOME`, `gpt-5.6-luna`, 2026-10-03 실행 환경 |
| H2 | `rules/default.rules`를 `forbidden`에서 `allow`로 바꿔도 같은 process의 다음 턴에는 직접 반영되지 않으며, `config/batchWrite(reloadUserConfig=true)` 또는 규칙 전용 공식 요청이 있더라도 반영 여부는 행동으로 확인된다. 반대 방향도 같은 기준으로 판정한다. | Codex app-server, `approvalPolicy=untrusted`, 읽기 전용 sandbox, `approvals_reviewer="user"`, 전용 `CODEX_HOME` |

## 설계

| 항목 | 값 |
|---|---|
| 조건 | 질문: 초기 feature 켜짐, 같은 process/thread에서 끔, 다시 켬. execpolicy: `forbidden → allow`의 (a) 무요청, (b) `config/batchWrite(reloadUserConfig=true)`, (c) 발견된 규칙 재읽기 요청, 새 process 대조; `allow → forbidden`도 같은 경로와 새 process에서 수행 |
| 배정 | 조건마다 trial 1, 2, 3의 고정 순서로 실행한다. 입력과 요청 순서는 모든 trial에서 같다. 무작위화하지 않는다 |
| 눈가림 | 해당 없음. driver가 설정 파일과 요청 경로를 알아야 한다. 판정은 원문 이벤트와 marker 파일·승인 요청을 대조한다 |
| 환경 | macOS, Python 3.9 이상, Saturn worktree, 설치된 Codex CLI 실제 버전, `gpt-5.6-luna`. 전용 `CODEX_HOME`은 worktree `.runtime/` 아래에 만들고 `~/.codex/auth.json`만 심볼릭 링크한다. `config.toml`은 `approval_policy = "untrusted"`, `sandbox_mode = "read-only"`, `approvals_reviewer = "user"`를 포함한다 |
| 공통 안전 | worktree 안의 무해한 `touch`와 `ls`만 유도한다. 로그인 원본을 복사·이동·수정하지 않고, 토큰과 인증값은 raw/private log 모두 가린다. provider process는 trial 종료 때 종료한다 |
| 읽기 방식 | app-server stdout은 `readline()` 대신 비차단 파일 descriptor를 바이트로 읽고, 한 번 읽은 chunk에서 생긴 모든 줄을 큐에 넣으며 원시 바이트 수신 시각을 기록한다 |

### 질문 기능 실행

각 trial은 feature가 켜진 전용 home으로 app-server를 한 번 띄우고 thread를 연다. 첫 입력은 사용자에게 두 선택지 중 하나를 물으라고 유도하여 `item/tool/requestUserInput`을 확인한다. 같은 process·thread에서 `experimentalFeature/enablement/set`으로 feature를 끄고 같은 유도 입력을 보내 요청이 오지 않는지 확인한다. 이어 같은 요청으로 feature를 켜고 같은 입력을 보내 다시 요청이 오는지 확인한다. `requestUserInput`이 오면 driver는 안전한 선택지 답을 보내고, 오지 않으면 턴 완료·timeout을 기록한다.

### execpolicy 실행

각 trial은 `prefix_rule(pattern = ["touch"], decision = "forbidden")`으로 시작한다. 같은 worktree 안의 marker에 `touch`를 실행하라는 입력을 보내 marker가 생기지 않고 승인 요청도 없는지 대조한다. 그 뒤 `default.rules`를 `allow`로 바꾸고 다음을 각각 별도 턴으로 실행한다: (a) 아무 요청도 보내지 않음, (b) `config/batchWrite`에 `reloadUserConfig=true`를 넣음, (c) schema에 규칙·execpolicy를 다시 읽는 공식 요청이 있으면 그것. 각 턴은 marker 생성 또는 승인 요청을 행동 근거로 기록한다. 이후 같은 process에서 규칙을 `forbidden`으로 되돌리고 (a), (b), (c)를 반복한다. 마지막으로 각 방향을 새 process에서 확인한다. read-only sandbox 때문에 allow가 승인 요청으로 나타나는 경우는 `approval`로 기록하고, accept 뒤 marker 생성 여부를 별도로 기록한다.

## 변수

| 변수 | 종류 | 정의 | 단위 |
|---|---|---|---|
| `trial_id` | 조작 | 조건별 1, 2, 3 | 없음 |
| `feature_state` | 조작 | `on`, `off` | 값 목록 |
| `rule_decision` | 조작 | `forbidden`, `allow` | 값 목록 |
| `reload_path` | 조작 | `none`, `config_batch`, 발견된 규칙 재읽기 요청, `new_process` | 값 목록 |
| `process_id` | 측정 | 설정 변경 전후 app-server OS process id | 문자열 |
| `thread_id` | 측정 | 같은 thread 여부를 확인하는 id | 문자열 |
| `user_input_request` | 측정 | `item/tool/requestUserInput` 이벤트가 온 여부 | 불리언 |
| `approval_request` | 측정 | 명령 승인 요청 이벤트가 온 여부 | 불리언 |
| `marker_effect` | 측정 | `touch` marker 파일이 생긴 여부 | 불리언 |
| `turn_result` | 측정 | 턴 완료, 오류, timeout | 값 목록 |
| `classification` | 파생 | 3회 결과가 모두 같으면 `확인`, 다르면 `불안정`, 실행·구별이 안 되면 `확인 못 함` | 값 목록 |

## 표본

| 항목 | 값 |
|---|---|
| 출처 | 설치된 Codex CLI app-server와 worktree 안 고정 marker fixture의 원문 JSON-RPC 이벤트 |
| 크기 | H1은 trial 3회. H2의 각 reload path와 양방향은 trial 3회. 새 process 대조도 방향마다 trial 3회 |
| 크기 근거 | 사용자가 지정한 조건당 반복 수와 Codex 모델 호출 상한 |
| 중단 규칙 | Codex 모델 호출이 50회에 닿으면 즉시 수집을 멈춘다. 인증 오류·토큰 누출·worktree 밖 쓰기·연속 3회 process 시작 실패가 발생하면 해당 route를 `확인 못 함`으로 남기고 종료한다 |
| 반복과 예열 | 예열 없음. 한 trial 안의 전환 순서는 고정하고, fresh process는 같은 fixture를 새로 읽는 대조로만 사용한다 |

## 분석

| 가설 | 지표 | 방법 | 채택 기준 |
|---|---|---|---|
| H1 | feature 상태별 `user_input_request` | 각 trial의 `on → off → on` 이벤트 순서와 process/thread id를 대조한다 | 3회 모두 `옴 → 오지 않음 → 옴`이면 `확인`, 하나라도 다르면 `불안정`, 요청을 관측할 수 없으면 `확인 못 함` |
| H2 | 규칙 상태·reload path별 approval/marker 행동 | forbidden 대조와 allow/forbidden 변경의 같은-process·새-process 행동을 2×2로 기록한다 | 같은 process의 실제 행동이 3회 모두 새 규칙에 맞으면 해당 path `확인`, 일부만 그러면 `불안정`, marker·승인으로 구별하지 못하면 `확인 못 함` |

| 항목 | 규칙 |
|---|---|
| 제외 기준 | 제외하지 않는다. provider 실패, timeout, 원문 파싱 실패, fixture 오류, worktree 밖 쓰기는 실패 흐름으로 센다 |
| 실패한 실행 | 같은 trial을 재시도하지 않는다. private 원문과 종료 상태를 보존하고 다음 trial로 진행한다 |
| 다중 비교 | 수치 검정은 하지 않는다. 각 route의 3회 일치 판정만 사용한다 |

## 판정

| 결과 | 설계에 반영 |
|---|---|
| 확인 | 해당 실행 중 설정 경로를 Saturn의 즉시 적용 계약으로 기록한다 |
| 불안정 | 해당 Codex 버전에서 즉시 적용을 보장하지 않고 조건과 변동을 기록한다 |
| 확인 못 함 | fixture 또는 실행 제약으로 확인하지 못한 경계로 남기고 재실측 조건을 기록한다 |

## 탐색 분석

- `config/batchWrite` 외 app-server schema에 규칙·execpolicy를 다시 읽는 요청이 있는지
- 설정 변경 전후 `process_id`와 질문 실험의 `thread_id`가 유지되는지
- 승인 요청이 온 경우 요청 method, available decisions, accept 뒤 marker 효과
- 직접 파일 변경과 `reloadUserConfig` 요청의 응답 오류·readback 차이

## 타당성 위협

| 종류 | 위협 | 대응 |
|---|---|---|
| 내적 | 모델이 유도 질문이나 `touch`를 매번 정확히 실행하지 않을 수 있다. | 고정 입력, raw 이벤트, marker 효과, 승인 요청을 함께 기록하고 실행되지 않은 회차를 성공으로 세지 않는다 |
| 내적 | read-only sandbox가 execpolicy와 별개로 승인 요청을 만들 수 있다. | forbidden에서는 승인 없음·marker 없음 대조를 먼저 두고, allow에서는 승인과 marker를 별도 필드로 기록한다 |
| 내적 | 설정 변경 요청의 성공 응답이 실제 파일 재읽기를 뜻하지 않을 수 있다. | 다음 턴 행동과 새 process 대조만 판정에 사용한다 |
| 구성 | `touch` 하나가 모든 execpolicy 규칙 재읽기를 대표하지 않을 수 있다. | 결론 범위를 `touch` prefix_rule과 지정한 Codex 환경으로 제한한다 |
| 외적 | Codex CLI·모델·schema가 바뀌면 요청과 행동이 달라질 수 있다. | 실제 버전, schema 파일 목록/hash, 실행 날짜, 모델 호출 수를 기록한다 |
| 외적 | 전용 home은 일반 home과 설정·인증 저장 방식이 다르다. | Saturn과 같은 reviewer·approval·sandbox 값을 쓰고 auth는 심볼릭 링크만 사용한다 |
