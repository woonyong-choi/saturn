# 인계 패킷 목표와 남은 일 채우기 방식: 실험 설계

| 항목 | 값 |
|---|---|
| 이슈 | [#249](https://github.com/woonyong-choi/saturn/issues/249) |
| 관련 설계 | [맥락 정리](../../design/context-management.md) |
| 사전 데이터 | 기존 `handoff-packet-quality-v2`의 시나리오 생성기, 질문 형식, 채점, provider 실행 방식을 확인하고 시드 249로 새 시나리오를 만든다. provider 답, summary 답, 결과 데이터는 수집 전에 보지 않는다. |

## 질문

고정 구역의 목표와 끝나지 않은 일을 마지막 입력, 규칙, 요약 모델 중 어떤 방식으로 채울지 정한다. 세 방식이 같은 경쟁 구역을 받을 때 새 session의 작업 상태 질문 정답률과 총 토큰 비용을 비교한다.

## 가설

| 가설 | 예측 | 적용 범위 |
|---|---|---|
| H1 | `rule`의 정답률에서 `last-input`의 정답률을 뺀 값은 15%p 이상이다. 차이의 95% 군집 부트스트랩 신뢰구간 하한이 +15%p 이상이다. | 시드 249 합성 시나리오 24개, Claude Code Haiku 4.5와 Codex GPT-5.6 Luna, 조건 3개 |
| H2 | `summary`의 정답률에서 `rule`의 정답률을 뺀 값은 5%p를 넘지 않는다. 차이의 95% 군집 부트스트랩 신뢰구간 상한이 +5%p 이하이다. | H1과 같다 |

## 설계

| 항목 | 값 |
|---|---|
| 조건 | `last-input`: 목표에 마지막 사용자 입력 하나를 넣고 남은 일을 비운다. `rule`: 목표에 첫 입력과 마지막 입력을 넣고 대기·보류·결과 미확인 입력을 남은 일로 넣는다. `summary`: `gpt-5.6-luna`가 기록을 읽고 같은 두 필드를 작성한다. 세 조건의 제약, 최근 턴, 경쟁 구역과 패킷 예산은 같다. |
| 배정 | 같은 시나리오와 질문을 세 조건과 두 provider에 대응시킨다. 시나리오 순서는 시드 249로 섞고, 시나리오 안의 조건 순서는 시나리오 id와 시드 249로 섞는다. 질문 순서는 시드 249로 섞고 세 조건에서 유지한다. |
| 눈가림 | 새 session provider에는 조건 이름을 보내지 않는다. 채점은 조건 이름을 보지 않고 정답 목록으로 한다. |
| 환경 | macOS, Python 3.9 이상 표준 라이브러리, Rust 패킷 예제, Claude Code(`claude -p --model claude-haiku-4-5-20251001`), Codex(`codex exec -m gpt-5.6-luna`), summary model(`gpt-5.6-luna`). 도구와 버전은 수집 때 `env.json`에 기록한다. provider와 summary model의 샘플링은 고정하지 않는다. |
| 실행 조건 | 로그인된 Claude와 Codex, `cargo`, `git`이 필요하다. provider session은 도구를 끄고 연다. summary model은 읽기 전용·사용자 설정 무시·규칙 무시로 연다. 사용자 전역 설정과 로그인 파일은 수정하지 않는다. |

- 패킷은 `saturn-terminal/core/examples/packet`으로 만든다. 세 조건 모두 경쟁 구역은 `--condition rrf-only`로 고정한다.
- B와 C의 고정 구역 입력은 패킷 예제의 `--fixed-file` JSON 옵션으로 전달한다. 제품 코드 `packet.rs`의 채우기 동작은 바꾸지 않는다.
- summary model은 시나리오마다 한 번 호출하고 같은 결과를 두 provider에 재사용한다. 호출 입력과 출력 토큰을 C의 비용에 더한다.
- provider는 시나리오 24개 × 조건 3개 × provider 2개로 144 session을 목표로 한다. 재시도까지 포함한 provider 호출 상한은 150회, summary 호출 상한은 30회다. 상한에 닿으면 수집을 멈춘다.

## 변수

| 변수 | 종류 | 정의 | 단위 |
|---|---|---|---|
| `condition` | 조작 | `last-input`, `rule`, `summary` | 없음 |
| `provider` | 조작 | `claude`, `codex` | 없음 |
| `qtype` | 조작 | `goal`, `recent`, `open`, `unknown-result`, `contrast` | 없음 |
| `correct` | 측정 | 정답 규칙을 만족하면 1, 아니면 0 | 없음 |
| `packet_tokens` | 측정 | 패킷 글자 수를 4로 나눈 값 | 토큰 |
| `prompt_tokens` | 측정 | provider 전체 질문 prompt 글자 수를 4로 나눈 값 | 토큰 |
| `summary_tokens` | 측정 | summary model 입력과 출력 토큰의 합 | 토큰 |
| `provider_calls` | 측정 | retry를 포함한 provider subprocess 호출 수 | 회 |
| `summary_calls` | 측정 | summary model subprocess 호출 수 | 회 |
| `accuracy` | 파생 | 조건별 `correct` 합 / 시행 수 | % |
| `diff_a` | 파생 | `accuracy(rule) - accuracy(last-input)` | %p |
| `diff_b` | 파생 | `accuracy(summary) - accuracy(rule)` | %p |
| `total_tokens` | 파생 | 시나리오 단위로 두 provider prompt 토큰 합과 summary 토큰을 더한 값 | 토큰 |

정답은 대소문자와 공백을 제거해 비교한다. 일반 질문은 모든 `gold` 값이 답에 있고 `unknown`이 거짓이며 `stale` 값이 없으면 맞는다. `unknown-result`의 두 번째 질문은 `모른다`, `확인 전`, `unknown` 중 하나를 포함해야 맞는다. 고정 구역 질문은 첫 입력, 마지막 입력, 대기·보류·결과 미확인 입력의 원문에 있는 값을 정답으로 둔다.

## 표본

| 항목 | 값 |
|---|---|
| 출처 | `01-collect`가 기존 실험의 생성 규칙을 재사용해 시드 249로 만드는 합성 시나리오 24개 |
| 크기 | 조건마다 480 시행(시나리오 24 × provider 2 × 질문 10) |
| 크기 근거 | 예산. provider 144회와 summary 24회를 상한 안에 두면서 기존 실험과 같은 질문 수를 유지한다. 24개 시나리오 군집의 차이 신뢰구간을 함께 보고 정밀도를 제한으로 기록한다. |
| 중단 규칙 | 24개 시나리오를 모두 실행하면 멈춘다. provider 호출 150회 또는 summary 호출 30회에 닿으면 즉시 멈추고 그때까지를 보고한다. 연속 provider 실패 5회면 멈춘다. |
| 반복과 예열 | session마다 1회, 예열 없음. provider 오류는 한 번 재시도한다. |

시나리오마다 첫 목표, 중간 지시, 마지막 지시, 대기 입력, 보류 입력, 결과 미확인 작업을 넣는다. 기존 실험의 extract, update, abstain과 유사한 대조 질문을 `contrast`로 유지하고, 고정 구역에 직접 답이 있는 질문과 경쟁 구역의 사실을 묻는 질문을 섞는다.

## 분석

| 가설 | 지표 | 방법 | 채택 기준 |
|---|---|---|---|
| H1 | `diff_a` | 시나리오 단위 군집 부트스트랩 10,000회, 시드 249, 95% 백분위 신뢰구간. 보조로 대응 2×2 표와 McNemar 검정을 보고한다. | Holm 보정 p ≤ 0.05이고 신뢰구간 하한 ≥ +15%p면 채택. 상한 < +15%p이고 p ≤ 0.05면 기각. 그 밖에는 보류. |
| H2 | `diff_b` | H1과 같은 방법 | Holm 보정 p ≤ 0.05이고 신뢰구간 상한 ≤ +5%p면 채택. 하한 > +5%p이고 p ≤ 0.05면 기각. 그 밖에는 보류. |

| 항목 | 규칙 |
|---|---|
| 제외 기준 | provider가 도구를 호출한 session, 종료 코드가 실패한 session, 답 JSON을 읽지 못한 session을 해당 시나리오·provider의 세 조건에서 함께 제외한다. summary 실패가 있으면 수집을 멈추고 제외하지 않는다. |
| 실패한 실행 | provider는 한 번 재시도한다. 재시도도 실패하면 시나리오·provider 단위를 세 조건에서 제외하고 흐름 표에 센다. |
| 다중 비교 | H1과 H2에 Holm-Bonferroni를 적용한다. |
| 비율 | 조건 정답률은 `k/n`과 95% Wilson 신뢰구간으로 쓴다. |
| 토큰 | 조건별 정확한 합과 평균을 쓴다. C의 summary 입력·출력 토큰은 시나리오마다 한 번만 더한다. |

### 채택 규칙

H1이 채택되면 B 또는 C를 A 대신 채택할 근거가 생긴다. H2가 채택되면 C의 정답률 상승이 +5%p 이내이므로 총 토큰 비용이 같아도 더 적은 B를 채택한다. H2가 기각되면 C의 차이 신뢰구간 하한이 +5%p보다 큰 경우에만 C를 검토하며, C의 `total_tokens / rule total_tokens`가 2 이하이면 C를 채택하고 2보다 크면 보류한다. H1 또는 H2가 보류되면 결론을 보류한다.

## 판정

| 결과 | 설계에 반영 |
|---|---|
| H1·H2 채택 | `context-management.md`의 고정 구역 요구사항에 `rule` 방식을 반영하고 이 보고서를 링크한다. |
| H1 채택·H2 기각·비용 기준 충족 | `summary` 방식을 반영하고 summary model 호출 비용을 설계에 기록한다. |
| H1 기각 또는 비용 기준 초과 | 현재 구현인 `last-input`을 유지하고 고정 구역 설계를 다시 정하는 design 이슈를 연다. |
| 보류 | 시나리오를 늘리지 않고 이번 결과의 실패 유형과 provider 차이를 검토할 후속 실험 이슈를 연다. |

## 탐색 분석

- 질문 유형별·provider별 조건 정답률과 `diff_a`, `diff_b`
- 고정 구역 질문과 경쟁 구역 질문의 정답률
- 조건별 패킷 토큰, prompt 토큰, summary 토큰, 총 토큰, 포함 항목 수
- summary model이 만든 goal과 open_items의 seq 보존율
- 기존 `handoff-packet-quality-v2`와의 기술 통계 차이

## 타당성 위협

| 종류 | 위협 | 대응 |
|---|---|---|
| 내적 | 한 scenario의 질문 10개가 같은 packet을 읽으므로 시행이 독립이 아니다. | 판정 신뢰구간을 scenario 단위 군집 부트스트랩으로 계산한다. |
| 내적 | provider와 summary model의 샘플링이 고정되지 않는다. | 같은 시나리오의 조건을 무작위 순서로 실행하고 모델·버전을 `env.json`에 남긴다. |
| 구성 | 문자열 포함 채점은 같은 의미의 다른 표현을 틀릴 수 있다. | 숫자와 고정 구역 핵심어를 정답 목록에 넣고 적용 범위를 합성 질문으로 제한한다. |
| 구성 | summary model이 만든 문장의 품질과 모델 호출 비용을 함께 측정한다. | 출력 JSON의 seq와 원문 보존을 검증하고 입력·출력 토큰을 별도 기록한다. |
| 외적 | 한 가상 서비스와 두 provider의 한 모델 버전만 사용한다. | 결론의 적용 범위를 이 실험 환경으로 제한한다. |
