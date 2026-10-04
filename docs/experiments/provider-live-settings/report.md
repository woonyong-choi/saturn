# provider 설정 즉시 적용 실측 결과

## 결론

실험 환경은 Claude Code 2.1.287, Codex CLI 0.158.0, macOS, Python 3.9.6이었다. 실행 중 적용을 행동으로 확인한 것은 Claude의 stream-json `set_permission_mode`뿐이다. Codex app-server는 `turn/start` 덮어쓰기와 공식 config/feature reload 요청을 제공해 요청이 성공하고 설정 readback도 바뀌었지만, 이 실험은 그 요청이 실행 중 행동을 바꾸는지 입증하지 못했다. 후속 [codex-live-reload](../codex-live-reload/report.md)는 `experimentalFeature/enablement/set`이 성공하고 feature list도 바뀌는데 질문 요청 행동은 같은 thread에서 3/3 바뀌지 않음을 보였다. 따라서 Codex의 runtime 요청은 "있다"까지만 확인된 경로이고, 매 턴 재시작이 필요 없다는 결론은 Claude permission mode에만 적용한다. 직접 파일을 바꾸는 방식의 execpolicy 재읽기는 이 fixture로 구별하지 못했다.

| provider | 다시 시작 없이 바꿀 수 있는 설정·경로 | 다음 턴부터 바뀌는 설정 | 다시 시작이 필요한 설정 또는 확인되지 않은 경계 |
|---|---|---|---|
| Claude Code | `control_request`의 `set_permission_mode`; 호출별 `can_use_tool` 응답으로 특정 도구의 이번 요청을 `deny` 또는 `allow` | `set_permission_mode`의 `dontAsk` 효과는 다음 턴 Bash 호출에서 확인. `set_model`은 control response는 성공했지만 실제 모델 변경 행동은 확인하지 못함 | `--settings`, `--allowedTools`, `--disallowedTools`, `--tools`, 시작 시 `--permission-mode`는 실행 인자. 실행 중 도구 허용·거부 목록을 바꾸는 확인된 control subtype 없음. `AskUserQuestion`은 모델의 재시도 변동으로 3회 결과가 불안정 |
| Codex app-server | 요청은 성공하지만 실행 중 적용은 입증하지 못한 경로: `turn/start`의 `approvalPolicy`, `sandboxPolicy` 등 override; `config/batchWrite`의 `reloadUserConfig=true`; `experimentalFeature/enablement/set`(후속 실험에서 같은 thread 행동 무효); `config/mcpServer/reload` | schema 설명상 `turn/start` override는 해당 턴과 subsequent turns에 적용. `config/batchWrite`는 loaded thread에 runtime config를 hot-reload하며 feature는 process-wide. 실측에서 `approval_policy=never`, `tool_suggest=false`가 다음 요청의 readback에 나타남 | session-static model·reasoning effort·Plan-mode effort·service tier 기본값·deprecated personality는 `reloadUserConfig` 대상에서 제외. `config.toml`·`rules/default.rules` 외부 직접 수정은 같은 process와 재시작 모두 fixture가 이미 명령을 실행해 차이를 판정하지 못함. rules 전용 live reload는 확인 못 함 |

적용안은 Claude permission mode에 한해 재시작 대신 공식 control request를 호출하는 것이다. Codex의 runtime 요청은 행동 효과를 따로 확인하기 전에는 적용 근거로 쓰지 않는다. Claude의 실행 인자와 Codex rules 파일처럼 provider 시작 시 읽는 경로는 별도 경계로 남겨야 한다.

## 설계와 다른 점

- 사전 가설 H2와 달리 Claude의 `set_model` control subtype은 세 번 모두 성공 응답을 받았다. 고정 모델 호출 조건을 지키기 위해 다른 모델로 전환하지 않았으므로 실제 모델 변경 효과는 확인하지 않았다.
- H4와 H5의 사전 예측은 Codex 생성 schema와 달랐다. `turn/start` 문서는 override가 현재 턴뿐 아니라 subsequent turns에도 적용된다고 설명했고, `config/batchWrite`에는 `reloadUserConfig` hot-reload가 있었다.
- 최초 inventory 수집의 schema 명령은 출력 경로 처리 오류로 실패했다. 이후 `codex app-server generate-json-schema --out <worktree>/.runtime/schema-inventory`로 다시 생성해 확인했다. 이 schema 산출물과 provider 원문은 메인 저장소 `.local/experiments/provider-live-settings/`에 보관했다.
- 초기 Claude 실행 두 번은 control request가 init event보다 먼저 기록되는 driver 순서 문제와 `auto` mode의 모델 제한(`auto mode unavailable for this model`)으로 중단했다. 해당 회차는 최종 route 판정에 포함하지 않고 private log와 흐름 기록에 남겼다.
- Codex 모델 호출 상한은 파일 비교 보강 재실행 때문에 실제 42회가 되었고, 요청 상한 40회를 2회 초과했다. 이를 확인한 뒤 Codex 모델 호출을 중단했다. Claude는 23회였다. 40회에 닿으면 멈추는 사전 중단 규칙을 지키지 못한 점은 한계다.
- 분류 기준을 고쳤다. 처음에는 세 회차 결과가 같으면 `확인`으로 분류해 성공 응답과 readback만 있는 route도 `확인`이 되었다. 이제 실행 중 적용을 행동으로 입증하지 못한 route는 `확인 못 함`이다(`scripts/03-analyze.py`의 `RESPONSE_ONLY`, 재집계한 `results/summary.json`). 원자료는 바꾸지 않았고 `data/SHA256SUMS`도 그대로다.

## 방법

Claude는 `claude -p --input-format stream-json --output-format stream-json --permission-prompt-tool stdio`를 새 process마다 실행했다. stdout은 비차단 바이트 읽기와 줄 큐로 수집했고, private log에는 방향·수신 시각·redacted message를 남겼다. `set_permission_mode`는 첫 턴에서 Bash를 거부한 뒤 같은 process의 다음 턴에서 `dontAsk`로 바꾸고 marker 파일과 `can_use_tool` 발생 여부를 확인했다. control 후보는 `set_model`, `set_max_thinking_tokens`, `mcp_message`, `rewind_files`, `stop_task`를 각각 세 번 보냈다. `AskUserQuestion`은 두 연속 요청에 deny·allow 응답을 시도했다.

Codex는 매 trial 전용 `CODEX_HOME`을 worktree `.runtime/codex-homes/`에 만들고 `~/.codex/auth.json`은 심볼릭 링크로만 연결했다. app-server stdout은 비차단 바이트 읽기와 줄 큐로 처리했다. `turn/start` override, `config/batchWrite`, `experimentalFeature/enablement/set`, `config/mcpServer/reload`를 실제 요청으로 보냈고, marker 파일·thread event·`config/read`/feature list 응답을 함께 확인했다. `config.toml`과 `rules/default.rules`는 process 실행 뒤 직접 바꾸고 같은 thread의 다음 턴과 새 process의 첫 턴을 비교했다.

큰 원문은 메인 저장소의 비공개 실험 로그에 있고, 공개 raw에는 비밀값과 사용자 절대 경로를 redaction한 요약 행만 둔다. 분석 재현 명령은 `./run.sh process`, `./run.sh analyze`, `./run.sh verify`다.

## 반복 결과

분류는 route별 세 trial의 결과가 모두 같고 실행 중 적용이 행동으로 드러나면 `확인`, 다르면 `불안정`, 실행은 되었지만 fixture가 설정 적용을 구별하지 못하거나 성공 응답·readback만 있으면 `확인 못 함`으로 했다.

| route | trial 결과 | 분류 | 행동 근거 |
|---|---:|---|---|
| Claude `set_permission_mode(dontAsk)` | 3/3 success | 확인 | 첫 Bash는 deny 후 marker 없음, 변경 후 다음 턴은 `can_use_tool` 없이 marker 없음 |
| Claude `set_model` | 3/3 success | 확인 못 함 | control response만 성공. 고정 모델 조건에서 실제 모델 변경을 검증하지 않음 |
| Claude 기타 control 후보 | 각 3/3 동일한 unsupported/error | 확인 | 지원하지 않는다는 오류가 3회 같았고 설정·도구 목록의 실행 중 행동 변경은 없음 |
| Claude `AskUserQuestion` deny → allow | `deny→allow`, `allow→allow`, `deny→deny` | 불안정 | 같은 process에서 per-request 응답은 가능했지만 모델의 실제 재호출 순서가 달랐음 |
| Codex `turn/start.approvalPolicy=never` | 3/3 success, 다음 턴 응답 | 확인 못 함 | 동일 thread에서 두 턴 모두 완료. approval request가 발생하지 않아 승인 차이를 입증하지 못함 |
| Codex `turn/start.sandboxPolicy=readOnly` | 3/3 marker 미생성, approval event는 변동 | 불안정 | read-only 효과는 일치했지만 approval event가 2회와 1회로 달랐음 |
| Codex `config/batchWrite(reloadUserConfig=true)` | 3/3 success, readback `never` | 확인 못 함 | config write가 `ok`, `config/read`가 `never`, 다음 턴 완료. 설정값 readback이고 실행 중 행동 변화는 입증하지 못함 |
| Codex `experimentalFeature/enablement/set` | 3/3 success, `tool_suggest=false` | 확인 못 함 | 같은 process의 feature list가 false로 바뀜. MCP reload 요청도 세 번 응답됨. 응답과 목록 값뿐이며, 후속 [codex-live-reload](../codex-live-reload/report.md)에서 같은 요청이 질문 행동을 3/3 바꾸지 못함 |
| Codex 직접 `config.toml` 수정 | 3/3 같은 process marker 생성, 재시작 marker 생성 | 확인 못 함 | 변경 전 marker도 이미 생성되어 전후 차이를 식별하지 못함 |
| Codex 직접 `rules/default.rules` 수정 | 3/3 같은 process marker 생성, 재시작 marker 생성 | 확인 못 함 | 규칙 효과를 구분할 수 있는 fixture가 아니었음. rules live reload 부재로 단정하지 않음 |

## 공식 경로와 Saturn 적용 경계

Codex schema에서 확인한 관련 요청은 `config/read`, `config/value/write`, `config/batchWrite`, `config/mcpServer/reload`, `experimentalFeature/list`, `experimentalFeature/enablement/set`이다. `config/batchWrite`의 `reloadUserConfig` 설명은 loaded thread에 runtime settings를 다시 넣되 session-static model, reasoning effort, Plan-mode reasoning effort, service-tier defaults, deprecated personality는 다시 읽지 않는다고 명시한다.

반면 현재 Saturn engine은 provider process와 app-server의 기본 시작·turn 경로를 사용하지만, 이번 실험에서 확인한 runtime 요청을 제품 코드에 연결하지는 않았다. 그러므로 이번 결과는 “현재 Saturn이 즉시 적용한다”가 아니라 “provider 공식 경로가 있고, engine 연결이 남은 구현 과제”라는 의미다.

## 재현·파일

- 설계: [design.md](design.md)
- raw 요약: [data/raw/](data/raw/)
- 정규화 결과: [data/processed/trials.csv](data/processed/trials.csv)
- 판정: [results/summary.json](results/summary.json)
- 수집 스크립트: [scripts/01-collect.py](scripts/01-collect.py), [scripts/common.py](scripts/common.py)
- 무결성: [data/SHA256SUMS](data/SHA256SUMS)
