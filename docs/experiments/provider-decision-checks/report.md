# 결정 전 provider 네 가지 실측 결과

사전 등록 설계: [design.md](design.md)

| 항목 | 값 |
|---|---|
| 이슈 | [#252](https://github.com/woonyong-choi/saturn/issues/252) |
| 수집일 | 2026-10-02 |
| 환경 | Claude Code 2.1.285(실험 1)·2.1.287(TUI 재수집) / `claude-haiku-4-5-20251001`, Codex CLI 0.158.0 / `gpt-5.6-luna`, Python 3.9.6 |
| 분석 원자료 호출 | Claude 51회, Codex 55회 |
| 전체 호출 시도 누계 | Claude 69회, Codex 69회(재수집 실패·인증 오류 포함) |
| 원자료 | `data/raw/` 및 `.local/experiments/provider-decision-checks/` |

## 설계와 다른 점

- 실험 1은 기존 16개 시나리오를 다시 수집하기 전에 두 조건의 포함 항목과 토큰 차이를 확인했다. 16개 모두 포함 항목 차이가 있어 예산을 줄이지 않았고, `tiktoken`이 설치되지 않아 `chars/4` 근사를 사용했다.
- 실험 2는 무경고 조건의 공통 상태 확인 지시를 제거한 뒤 전수 20회를 재수집했다. 분석에는 최신 run만 사용했다.
- 실험 4는 stream-json/app-server 수집 대신 tmux에서 실제 Claude Code와 Codex CLI TUI를 열고 화면을 저장했다. Claude는 `--permission-mode manual`을 명시했고, Codex는 기존 세션의 업데이트 창을 `2`와 `Enter`로 닫으려 했다.
- 실험 4의 최신 완료 run은 provider·상태별 3회 화면을 남겼지만 Claude는 각 조건에서 명령 실행이 1회뿐이었고 Codex는 전 회차 인증 오류로 명령을 실행하지 못했다. 추가 시도는 이슈를 확인한 뒤 중단했으며, 새 로그인은 하지 않았다.
- 설계의 Claude Code 2.1.285와 달리 실제 수집 환경은 2.1.287이었다. 이 차이는 결과 적용 범위를 제한한다.

## 실험 1. 맥락 초과 줄이기

시드 252의 같은 16개 시나리오 × 2 provider × 2 조건으로 64 시행, 조건별 320문항이다. 수집 전 조작 확인에서 16개 시나리오 모두 포함 항목 차이가 발생했다.

| 조건 | 정답 | 정답률 | 95% CI |
|---|---:|---:|---|
| `oldest-first` | 64/320 | 20.0% | 15.98–24.73% |
| `tail-preserve` | 64/320 | 20.0% | 15.98–24.73% |

대응 비교는 `tail-preserve - oldest-first = 0.0%`, 95% 부트스트랩 구간 `[0.0%, 0.0%]`, 불일치 `b=0, c=0`이다. 패킷 토큰 상대 차이는 32쌍에서 평균 7.1%, 중앙값 9.2%, p5 −12.2%, p95 15.7%, 95% 부트스트랩 구간 `[2.8%, 11.2%]`였다.

판정: H1은 지지하지 않고 H2는 기각한다. 조작 확인으로 두 전략의 입력은 달라졌지만, 정답률 우열은 없었고 `tail-preserve`의 토큰도 예측대로 줄지 않았다. 이 합성 표본만으로 Saturn의 전환 규칙을 결정할 근거가 없다.

### 조작 확인

| 시나리오 | 두 조건 포함 항목 차이 수 | `oldest-first` 토큰 | `tail-preserve` 토큰 | 토큰 차이 |
|---|---:|---:|---:|---:|
| s01 | 61 | 3,687 | 4,238 | +551 |
| s02 | 62 | 3,813 | 3,348 | −465 |
| s03 | 61 | 3,598 | 4,154 | +556 |
| s04 | 62 | 3,950 | 4,042 | +92 |
| s05 | 61 | 3,510 | 4,062 | +552 |
| s06 | 61 | 3,649 | 4,199 | +550 |
| s07 | 62 | 4,035 | 4,113 | +78 |
| s08 | 62 | 3,804 | 4,154 | +350 |
| s09 | 62 | 4,026 | 3,783 | −243 |
| s10 | 12 | 3,964 | 4,074 | +110 |
| s11 | 61 | 3,566 | 4,116 | +550 |
| s12 | 61 | 3,627 | 4,178 | +551 |
| s13 | 14 | 3,956 | 4,085 | +129 |
| s14 | 61 | 3,526 | 4,079 | +553 |
| s15 | 62 | 4,030 | 4,234 | +204 |
| s16 | 60 | 3,861 | 3,845 | −16 |

포함 항목 차이는 12~62개였고, 0개인 시나리오는 없었다. 토큰은 모두 `chars/4` 근사값이다.

## 실험 2. 결과를 모르는 작업 이어 가기

provider·조건별 5회, 총 20회다. 실제 명령 이벤트를 private raw log에서 다시 추출했다.

| 조건 | 상태 확인 먼저 | 중복 `touch` |
|---|---:|---:|
| 문구 없음 | 5/10 (50.0%, CI 23.66–76.34%) | 5/10 (50.0%, CI 23.66–76.34%) |
| 상태 확인 경고 | 5/10 (50.0%, CI 23.66–76.34%) | 1/10 (10.0%, CI 1.79–40.42%) |

대응 표본에서 상태 확인의 불일치는 0건이었다. 중복 `touch`는 경고 조건에서 4건 줄었지만, 상태 확인 먼저라는 사전 판정 기준을 충족하지 못했다.

판정: H3 보류. 경고 문구가 같은 명령 재실행은 줄였지만, 첫 명령을 상태 확인으로 바꾸지는 않았다. 다음 설계에서는 “상태 확인”을 별도 요구하지 않는 순수 재개 문구와 파일 존재 확인을 독립 지표로 유지해야 한다.

## 실험 3. 입력 요청 형식과 왕복

Codex 7회, Claude 3회다. 모든 기록된 요청은 드라이버 응답 후 왕복 지표가 1이었다.

| provider / 원문 형식 | 요청 기록 | 왕복 |
|---|---:|---:|
| Codex `mcpServer/elicitation/request` | 8 | 8 |
| Codex `item/tool/requestUserInput` | 1 | 1 |
| Claude `control_request` (`AskUserQuestion`) | 3 | 3 |

Codex form 요청에는 문자열·정수·불리언·단일 선택·다중 선택 schema가 원문으로 남았다. URL 요청에는 `url`과 `elicitationId`가 남았다. Codex agent request는 `default_mode_request_user_input` 활성화 뒤 `questions` 배열과 `isBlocking`을 전달했고, 응답은 `answers` 객체였다. Claude는 `control_request.request.subtype=can_use_tool`, `tool_name=AskUserQuestion`으로 왔고, `updatedInput.answers` 응답 뒤 후속 출력에서 답을 반영했다.

판정: H4 확인. Saturn의 adapter는 provider별 원문을 보존하고, Codex는 MCP elicitation과 agent user-input을 별도 계약으로 다뤄야 하며, Claude는 stream-json control request와 `updatedInput.answers`를 별도 처리해야 한다.

## 실험 4. 중단·강제 종료 표시

최신 완료 run `20261002T152523Z-a18c242`에서 provider·상태별 3회씩 총 12회 화면을 저장했다. 명령이 실제 실행된 회차와 화면 원문은 다음과 같았다.

| provider / 상태 | 실행 회차 | 화면 원문 | 판정 |
|---|---:|---|---|
| Claude / 정상 중단 | 1/3 | `Running in the background (↓ to manage)`; `Interrupted · What should Claude do instead?` | 미확인 |
| Claude / 강제 종료 | 1/3 | `Running in the background (↓ to manage)`; `Interrupted · What should Claude do instead?` | 미확인 |
| Codex / 정상 중단 | 0/3 | `Your access token could not be refreshed because you have since logged out or signed in to another account. Please sign in again.` | 미확인 |
| Codex / 강제 종료 | 0/3 | `Update available ...`; 명령 실행·중단 문구 없음 | 미확인 |

Claude의 원문 화면은 `data/raw/exp4-20261002T152523Z-a18c242-claude-*.txt`에, Codex 화면은 같은 실행 ID의 `codex-*.txt`에 저장했다. Claude의 유효 회차에서 정상 중단과 강제 종료 화면이 같은 문구였고, 조건별 3회 유효 표본과 provider별 공통 구분 문구를 확보하지 못했다. Codex는 인증 오류로 로그인 없이 중단했으므로 H5는 미확인이다.

## 종합 판정

1. 맥락 정리 두 전략은 이번 합성 표본에서 우열이 없었다. 실제 packet compiler/tokenizer 연결 뒤 다시 측정할 때까지 선택을 보류한다.
2. 상태 경고는 중복 명령을 줄이는 신호가 있었지만 첫 상태 확인을 보장하지 않았다. 재개 계약에는 상태 확인 지시와 중복 실행 방지를 별도 규칙으로 둔다.
3. 입력 요청은 Codex MCP elicitation, Codex agent request, Claude control request가 서로 다른 원문 계약임을 확인했다.
4. TUI 화면 수집은 Claude의 `Interrupted` 문구 2개 유효 회차를 남겼지만 3회 반복과 Codex 실행이 성립하지 않았다. provider 공통 중단·강제 종료 문구는 여전히 미확인이다.

## 재현과 검증

```sh
./run.sh process
./run.sh analyze
./run.sh verify
```

수치의 원천은 `results/summary.json`이며 raw 무결성은 `data/SHA256SUMS`로 확인한다. 원문 로그는 저장소 `.local/experiments/provider-decision-checks/`에 보관했다.
