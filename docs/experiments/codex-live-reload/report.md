# Codex 실행 중 설정 다시 읽기: 실험 결과

## 요약

Codex CLI 0.158.0에서 질문 feature 토글은 RPC 성공 응답과 달리 실제 같은 thread 행동을 바꾸지 않았다: 켜짐 시작 3/3은 `옴 → 옴 → 옴`, 꺼짐 시작 3/3은 `오지 않음 → 오지 않음`이었다. execpolicy 파일 변경은 같은 process의 무요청과 `config/batchWrite(reloadUserConfig=true)`에서 모두 반영되지 않았고, 새 process의 allow만 marker 3/3을 만들었다. 질문 기능의 실행 중 토글은 `확인`(재현된 무효), execpolicy는 `확인`(새 process에서만 적용)으로 판정한다.

## 방법

| 항목 | 값 |
|---|---|
| 설계 | [실험 설계](design.md), 커밋 `a1851ce` |
| 실행 id | `20261003T042825Z-a1851ce`, `20261003T042952Z-a1851ce`, `20261003T043432Z-a1851ce`, `20261003T043720Z-a1851ce`, `20261003T043951Z-a1851ce` |
| 환경 | [env.json](env.json) |
| 표본 | 질문 상태 3회씩과 역방향 3회, execpolicy 경로·양방향 각 3회. 정규화 39행 |

각 질문 trial은 `[features] default_mode_request_user_input=true` 또는 `false`로 전용 `CODEX_HOME`을 만들고 하나의 app-server process와 thread를 유지했다. 질문 유도 입력 뒤 `experimentalFeature/enablement/set`으로 끄거나 켠 뒤 같은 입력을 보냈다. 질문 요청은 `item/tool/requestUserInput` event로, 답은 schema가 요구한 질문 id별 `{"answers":[글]}` 구조로 보냈다.

각 execpolicy trial은 `thread/start`에 `approvalPolicy="untrusted"`, `sandbox="read-only"`를 주고 config에 `approvals_reviewer="user"`를 두었다. `touch .runtime/markers/...`를 실행하게 한 뒤 `rules/default.rules`를 `forbidden`과 `allow` 사이에서 바꾸고, 무요청·`config/batchWrite(reloadUserConfig=true)`·새 process를 비교했다. marker 파일과 승인 요청 event를 함께 기록했다.

app-server stdout은 기존 실험과 같은 비차단 byte read와 줄 큐를 사용했다. 큰 원문은 메인 저장소 [private log](../../../../saturn/.local/experiments/codex-live-reload/)에 두었고, 공개 raw에는 redaction한 요약만 남겼다.

## 설계와 다른 점

| 변경 | 시점 | 이유 | 결론에 미친 영향 |
|---|---|---|---|
| `approval_policy="untrusted"`를 config에서 제거하고 `thread/start.approvalPolicy`로 이동 | 수집 중 | Codex 0.158.0이 config의 해당 키를 “no longer supported”로 거부했지만 app-server schema와 thread 응답은 `untrusted`를 지원했다 | Saturn 조건을 유지한 유효 execpolicy 수집을 가능하게 했다 |
| 첫 질문 수집을 raw에 보존하고 driver 수정 뒤 같은 trial id로 재수집 | 수집 중 | 첫 driver가 `threadId`를 enablement 요청에 넣었고 질문 답 구조도 잘못 보내 실제 off 결과를 검증할 수 없었다 | 첫 raw는 실패 흐름으로 남기고 최신 raw를 정규화에 사용했다 |
| 초기 설계에 없던 `꺼짐 시작 → 켜짐` 역방향을 3회 추가 | 수집 중 | 사용자가 요구한 반대 방향을 별도로 확인하기 위해 추가했다 | 역방향도 실행 중 토글 효과가 없다는 결과를 얻었다 |
| Codex 호출 상한을 4회 초과 | 수집 중 | driver 수정 재수집과 역방향 추가가 필요했다 | 실제 누적 54회에서 provider 수집을 중단했고, 초과를 결과에 명시한다 |

## 결과

### 흐름

| 단계 | 수 |
|---|---:|
| 수집 raw 실행 | 5 |
| 초기 execpolicy 시작 실패: config `approval_policy` 미지원 | 3 trial |
| 질문 on/off/on 상태 | 9 |
| 질문 off/on 역방향 상태 | 6 |
| execpolicy 유효 행동 상태 | 21 |
| schema inventory만 수집 | 2 |
| 분석 | 39 trial 행 |
| 호출 상한 초과 | 4회 |

초기 실패 3회는 제외하지 않았고 `codex.execpolicy.start`의 `확인 못 함` 흐름으로 남겼다. 결과 판정은 수정된 driver의 유효 trial을 사용했다.

### 확인 분석

| 가설 또는 경로 | 지표 | 값 | 3회 결과 | 판정 |
|---|---|---|---|---|
| H1 켜짐 시작 | 질문 요청 순서 | `옴 → 옴 → 옴` | 3/3 동일 | 확인: 끄기 요청이 실제 행동을 끄지 못함 |
| H1 꺼짐 시작 | 질문 요청 순서 | `오지 않음 → 오지 않음` | 3/3 동일 | 확인: 켜기 요청이 실제 행동을 켜지 못함 |
| H2 forbidden 대조 | 승인/marker | `0/3`, `0/3` | 3/3 동일 | 확인 |
| H2 allow, 무요청 | 승인/marker | `0/3`, `0/3` | 3/3 동일 | 확인: 같은 process에 미반영 |
| H2 allow, config batch | 승인/marker | `0/3`, `0/3` | 3/3 동일 | 확인: `reloadUserConfig`로도 미반영 |
| H2 forbidden, 무요청 | 승인/marker | `0/3`, `0/3` | 3/3 동일 | 확인: 같은 process가 이전 정책을 유지 |
| H2 forbidden, config batch | 승인/marker | `0/3`, `0/3` | 3/3 동일 | 확인 |
| H2 새 process allow | 승인/marker | `0/3`, `3/3` | 3/3 동일 | 확인: 새 process에서 allow 적용 |
| H2 새 process forbidden | 승인/marker | `0/3`, `0/3` | 3/3 동일 | 확인 |

schema inventory에는 `config/batchWrite`와 `config/read`가 있었고, 규칙·execpolicy를 직접 다시 읽는 이름의 schema는 없었다. 따라서 별도 (c) 요청은 실행할 수 없었고, `config/batchWrite`를 유일한 설정 재읽기 후보로 시험했다.

## 논의

### 해석

이 환경에서 `experimentalFeature/enablement/set`은 요청 응답을 성공으로 돌려주지만 `default_mode_request_user_input`의 같은 thread 실제 행동을 바꾸지 않았다. 따라서 #284의 “실행 중 질문을 끄고 켠다” 계약은 이 버전의 실제 provider 행동으로는 확인되지 않으며, 새 process 시작 설정은 여전히 질문 on/off를 구분했다. `rules/default.rules`도 app-server가 시작할 때 읽은 값이 유지됐고 `config/batchWrite(reloadUserConfig=true)`는 execpolicy를 다시 읽게 하지 않았다. 이 결과는 Saturn의 규칙 변경 재시작 경계를 유지하고, 질문 feature도 live toggle을 보장하려면 후속 버전별 재실측 또는 재시작 경로가 필요하다는 뜻이다.

### 타당성 위협

| 종류 | 위협 | 이 실험에서 |
|---|---|---|
| 내적 | 모델이 질문이나 touch를 호출하지 않을 수 있다 | 질문은 feature on/off 상태에 따라 요청 event가 일관되게 왔고, execpolicy 대조는 모델이 “blocked by execution policy”를 반환했으며 새 process allow에서 marker가 생겼다 |
| 내적 | read-only sandbox가 execpolicy와 별도로 승인 요청을 만들 수 있다 | 모든 execpolicy 상태에서 승인 요청은 0/3이었고 marker를 주 판정으로 사용했다 |
| 내적 | 성공 RPC가 실제 재읽기를 뜻하지 않을 수 있다 | 다음 턴 행동과 새 process 행동만 판정에 사용했다 |
| 구성 | touch 하나가 모든 규칙을 대표하지 않을 수 있다 | 결론을 `touch` prefix_rule과 Codex 0.158.0으로 제한했다 |
| 외적 | provider 버전과 schema가 바뀔 수 있다 | 버전 0.158.0, schema hash, 실행 날짜를 env에 기록했다 |
| 외적 | 전용 home이 일반 home과 다르다 | Saturn과 같은 reviewer·approvalPolicy·read-only 조건을 쓰고 auth는 심볼릭 링크만 사용했다 |

### 한계

- 사용자가 지정한 Codex model call cap 50회를 driver turn-start 기준 4회 초과했다. 이후 provider 호출은 하지 않았다.
- 모델의 실제 내부 API 호출 수가 app-server stdout에 별도 usage event로 노출되지 않아, 호출 수는 driver가 보낸 `turn/start` 요청 수로 정의했다.
- 별도 규칙 reload RPC가 schema에 없었으므로 해당 경로는 `확인 못 함`이 아니라 “발견되지 않아 실행하지 못함”으로 기록했다.

## 재현

```sh
./run.sh verify
./run.sh analyze
```

현재 worktree에서는 provider 수집 뒤 `.runtime/`을 제거했으므로 재현 명령은 raw·결과 재검증과 재분석에 한정한다. 새 provider 수집은 상한 초과 때문에 수행하지 않았다.

| 파일 | SHA-256 |
|---|---|
| `data/SHA256SUMS` | 수집 raw 5개 SHA-256 목록 |
| `results/summary.json` | `cd046bfa1c8e3a529cc428546c841bc79f52d80f0ab8055f70439a3b43451fe9` |

## 결론

| 가설 | 판정 | 반영한 문서 |
|---|---|---|
| H1 | 확인: live enablement RPC 성공과 실제 질문 행동이 일치하지 않음(켜짐 시작 3/3 옴·옴·옴, 꺼짐 시작 3/3 오지 않음·오지 않음) | [입력 요청](../../design/input-requests.md) |
| H2 | 확인: rules 파일과 config batch는 같은 process에서 재읽지 않고 새 process에서만 적용 | [권한](../../design/permissions.md) |
