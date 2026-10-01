# 변환 수정 뒤 Codex와 Claude 기록의 전환 품질: 실험 설계

| 항목 | 값 |
|---|---|
| 이슈 | [#219](https://github.com/woonyong-choi/saturn/issues/219) |
| 관련 설계 | [provider 연결과 session](../../design/providers-and-sessions.md), [맥락 고르기](../../design/context-selection.md), [맥락 정리](../../design/context-management.md) |
| 사전 데이터 | [record-fidelity](../record-fidelity/report.md)의 1단계 결과(C1 기각)와 원시 줄을 쓴다. 수정된 변환으로 이 원시 줄을 재생해 두 provider가 모두 거친 근거 사실이 작업 24개에서 312개임을 확인했고, 받는 쪽을 부르지 않고 패킷 예산 800, 1,200, 1,600, 2,400토큰에서 근거 사실이 패킷에 드는 수를 세어 예산을 정했다. 받는 쪽 답과 정답률은 보지 않았다. |

## 질문

[변환 수정](https://github.com/woonyong-choi/saturn/issues/203)([#210](https://github.com/woonyong-choi/saturn/issues/210), [#214](https://github.com/woonyong-choi/saturn/issues/214), [#215](https://github.com/woonyong-choi/saturn/issues/215)) 뒤에, 받는 쪽을 고정했을 때 Codex(GPT-6 Sol) 기록 패킷의 정답률이 Claude Code(Opus 5.5) 기록 패킷보다 10%p 넘게 낮지 않은지 잰다. [record-fidelity](../record-fidelity/design.md)의 2단계를 수정된 변환으로 실행한다.

## 가설

| 가설 | 예측 | 적용 범위 |
|---|---|---|
| C2 | 같은 받는 쪽에서 Codex 기록 패킷의 정답률은 Claude 기록 패킷보다 10%p 넘게 낮지 않다. 받는 쪽마다 차이(Codex − Claude)의 작업 단위 군집 부트스트랩 95% 신뢰구간 하한이 −10%p보다 크다. | 합성 작업 24개, 두 provider가 모두 거친 근거 사실, 받는 쪽 Opus 5.5와 GPT-6 Sol 새 session(도구 끔), 예산 1,600토큰의 새 패킷 규칙 |

## 설계

| 항목 | 값 |
|---|---|
| 조건 | 기록을 만든 provider(`codex`, `claude`) × 받는 쪽(`claude`, `codex`) 2×2. 받는 쪽을 고정하고 기록을 만든 provider만 바꾼다. |
| 배정 | 같은 작업 24개([record-fidelity](../record-fidelity/design.md)의 시드 199)의 두 기록을 같은 질문으로 묻는 대응 설계다. 세션 96개(작업 24 × 기록 2 × 받는 쪽 2)의 순서는 시드 199로 섞고, 받는 쪽마다 한 줄로 실행한다. |
| 눈가림 | 받는 쪽은 패킷을 만든 provider를 모른다. 프롬프트와 패킷에 provider 이름을 넣지 않는다. 채점은 `02-process`가 정답 값 포함 여부로 하고 기록을 만든 provider를 보지 않는다. |
| 환경 | macOS, Python 3.9 이상(표준 라이브러리), Rust(`cargo`), Claude Code `-p`, Codex CLI. 모델은 `claude-opus-5-5`와 `gpt-6-sol`. 도구와 모델 버전은 수집 때 `env.json`에 적는다. provider의 LLM 샘플링은 시드로 고정할 수 없다. |

기록과 패킷은 다음과 같이 만든다.

- 기록은 [record-fidelity](../record-fidelity/report.md)가 저장한 provider 원시 줄(`data/raw/codex-*.jsonl`, `claude-*.jsonl`)을 재사용한다. provider 작업은 다시 실행하지 않는다. 원시 줄은 provider가 낸 줄 그대로이고 Saturn 변환 코드에 의존하지 않으며, 수정된 변환이 읽는 값(Codex의 종료 코드, 수정 내용, 명령 동작, 추론 항목, Claude의 경로와 줄 수의 바탕이 되는 도구 입력)이 그 줄에 모두 있기 때문이다. 같은 작업과 지시문, 같은 버전의 provider로 얻은 줄이라 다시 실행해도 같은 종류의 줄이 나온다. 재사용의 한계는 한계 절에 적는다.
- 재생은 engine의 예제 `saturn-terminal/engine/examples/record-fidelity`를 `scripts/shim.py`의 재생 모드로 실행해 저장한 `out` 줄을 순서대로 돌려주는 방식이다. 수정된 변환이 낸 이벤트(`ProviderEvent`)가 분석의 Saturn 기록이다. 재생한 이벤트는 `raw/events-*.jsonl`에 저장한다.
- 기록에서 `Reasoning` 종류의 호출은 뺀다([#215](https://github.com/woonyong-choi/saturn/issues/215)가 도구 결과 후보에서 빼는 항목이다). 도구 기록은 이벤트의 도구 종류(`read`, `edit`, `shell`, `test`)와 경로, 명령, 종료 코드, 바뀐 줄 수를 패킷 예제의 인자로 옮기고, 결과 글은 이벤트의 출력이다. 두 provider에 같은 규칙이다.
- 패킷은 `saturn-core`의 `packet` 예제가 새 규칙(기준값 없이 확률 순으로 예산까지 채움, session과 기록 번호 표기)으로 만든다. 마지막 입력은 두 번째 사용자 입력으로 근거 사실 질문 목록이다. 판단은 근거 값이 든 첫 도구 기록의 확률을 1, 나머지를 0으로 하는 `oracle`이다. judge 판단 오차가 변환 차이에 섞이지 않게 하기 위해서다. 예산 `P_max`는 1,600토큰이다. 사전 데이터의 예산 시험에서 800토큰은 근거 사실이 두 기록 모두 절반 안팎(146/312, 143/312)만 들었고, 1,200토큰은 288/312, 1,600토큰은 312/312가 들었다. 근거 사실이 패킷에 들지 않으면 받는 쪽이 맞힐 수 없어 변환 차이를 잴 수 없다.
- 받는 쪽은 도구를 끈 새 session이다. Codex는 `--ephemeral --ignore-user-config --ignore-rules --sandbox read-only`, Claude Code는 `--safe-mode --tools "" --no-session-persistence`로 실행한다. 작업 폴더는 저장소 밖 오케스트레이션 폴더다. 동시에 실행하는 provider 세션은 2개 이하(받는 쪽마다 하나)다.
- 질문은 작업마다 두 provider가 모두 거친 근거 사실만이다. 한 provider의 원시 줄에서 도구 결과에 값이 있는 것으로 정한다. 질문 목록은 두 기록 패킷에서 같다.

## 변수

| 변수 | 종류 | 정의 | 단위 |
|---|---|---|---|
| `source` | 조작 | 기록을 만든 provider. `codex`, `claude` | 없음 |
| `receiver` | 조작 | 패킷을 받는 새 session. `claude`(Opus 5.5), `codex`(GPT-6 Sol) | 없음 |
| `correct` | 측정 | 받는 쪽의 답에 정답 값이 있으면 1, 아니면 0. 소문자로 바꾼 글의 포함 비교다. | 없음 |
| `in_packet` | 측정 | 근거 사실의 값이 패킷 글에 있으면 1 | 없음 |
| `packet_tokens` | 측정 | 패킷의 추정 토큰 수(글자 수를 4로 나눔) | 토큰 |
| `diff` | 파생 | 같은 근거 사실에서 Codex 기록의 `correct` − Claude 기록의 `correct`의 평균 | %p |

## 표본

| 항목 | 값 |
|---|---|
| 출처 | [record-fidelity](../record-fidelity/design.md)의 공개 합성 작업 24개(`scripts/tasks.py`, 시드 199)와 그 원시 줄. 작업마다 파일 읽기 4번, 명령 3번, 수정 1번이고 근거 사실이 13개다. |
| 크기 | 세션 96개. 받는 쪽마다 근거 사실 312개 × 기록 2(작업 24, 질문은 두 provider가 모두 거친 것). 실제 수는 수집 때 재생으로 정한다. |
| 크기 근거 | 예산과 정밀도. 근거 사실 200개 이상이 10%p 차이를 가르는 최소 크기다. 사전 데이터에서 312개이므로 충족한다. 군집은 작업 24개다. |
| 중단 규칙 | 근거 사실이 200개 미만이거나 작업당 평균 5개 미만이거나 작업이 20개 미만이면 받는 쪽을 돌리지 않고 보고한다. 연속 5개 세션이 실패하면 멈추고 고친 뒤 새 실행 id로 다시 수집한다. 사용률 규칙은 아래에 있다. |
| 반복과 예열 | 세션마다 1회, 예열 없음 |

사용률 규칙은 다음과 같다.

- 사용률은 Claude 주간(`claude -p "/usage"`의 `Current week (all models)`), Claude 5시간 창, Codex 주간(가장 최근 `~/.codex/sessions` 기록의 `rate_limits.primary.used_percent`, 창 10080분)이다.
- [#218](https://github.com/woonyong-choi/saturn/issues/218)과 합쳐 Claude 주간과 Codex 주간이 각각 #218 시작 값(Claude 27%, Codex 10%) 대비 5%p를 넘게 오르면 멈추고 보고한다. 시작 값은 `env.json`에 적는다.
- 작업 5개(세션 20개)를 먼저 받는 쪽에 넘겨 증가를 잰 뒤 전체 증가를 외삽한다. 읽은 증가 × 24/5가 한도를 넘기거나 판단이 애매하면 멈추고 보고한다. 이 5개는 본 수집에 포함한다. 배치(작업 4개)마다 사용률을 읽고, Claude 5시간 창이 85%를 넘으면 초기화될 때까지 기다린다.
- 같은 계정을 다른 작업도 쓰므로 사용률 증가에는 이 실험 밖의 사용이 섞인다.

## 분석

| 가설 | 지표 | 방법 | 채택 기준 |
|---|---|---|---|
| C2 | 받는 쪽마다 `correct`의 `diff` | 같은 근거 사실에서 Codex 기록 패킷과 Claude 기록 패킷의 대응 차이. 작업 단위 군집 부트스트랩(10,000회, 시드 219)의 95% 백분위 신뢰구간. 보조로 2×2 표, 불일치 수 b, c, Newcombe 대응 차이 신뢰구간 | 받는 쪽마다 하한 > −10%p면 채택, 상한 ≤ −10%p면 기각. 두 받는 쪽이 모두 채택이면 C2 채택, 하나라도 기각이면 C2 기각, 그 밖은 보류 |

| 항목 | 규칙 |
|---|---|
| 제외 기준 | 두 기록 중 하나의 세션이 다시 실행해도 실패한 작업은 그 받는 쪽에서 두 기록 모두 뺀다. 답 JSON을 읽을 수 없거나 도구를 부른 세션은 틀린 답으로 센다. |
| 실패한 실행 | provider 종료 코드가 0이 아니거나 300초를 넘으면 한 번 다시 실행한다. 다시 실패하면 제외하고 흐름 표에 센다. |
| 다중 비교 | 받는 쪽 둘의 교집합 판정이라 보정하지 않는다. |

- 정답률은 `k/n`, 비율, 95% Wilson 신뢰구간으로 쓴다.

## 판정

| 결과 | 설계에 반영 |
|---|---|
| 채택 | 현재 변환을 유지하고 [provider 연결과 session](../../design/providers-and-sessions.md) 요구사항 표의 이 실험 행 검증 계획을 보고서 링크와 수치로 바꾼다. |
| 기각 | 정답률이 낮은 원인(경로, 종료 코드, 수정 내용 중 패킷에서 빠진 값)을 보고서에 적고 변환 bug 이슈를 연다. |
| 보류 | 작업을 늘려 새 실행 id로 다시 수집하고, 정밀도가 모자란 받는 쪽을 보고서에 적는다. |

## 탐색 분석

- 수정된 변환으로 다시 잰 변환 충실도: 근거 항목 포착률, 도구 종류, 경로, 메모 필드 정확도의 Codex − Claude 차이(record-fidelity 1단계 지표와 같은 규칙)
- 근거 사실이 패킷에 든 비율과 조건별 패킷 토큰, 기록을 만든 provider별 받는 쪽 정답률
- 기록의 도구 종류별 수, 추론 항목 수, 명령 감싸기 수, 비어 있는 수정 결과 수

## 타당성 위협

| 종류 | 위협 | 대응 |
|---|---|---|
| 내적 | 근거 사실은 같은 작업 안에서 서로 기대므로 사실 단위 독립을 가정한 신뢰구간이 좁다. | 판정에 작업 단위 군집 부트스트랩을 쓰고 Newcombe 구간은 보조로만 보고한다. |
| 내적 | 받는 쪽의 샘플링이 고정되지 않아 같은 패킷도 다른 답을 낼 수 있다. | 세션 순서를 시드로 섞고 받는 쪽마다 두 기록을 같은 시간대에 실행한다. |
| 내적 | provider가 지시와 다르게 도구를 더 부르면 기록이 달라진다. | 두 provider가 모두 거친 근거 사실만 묻는다. |
| 구성 | `oracle` 판단은 근거 기록을 항상 넣어 실제 judge가 놓치는 경우를 재지 않는다. | 변환 차이만 재려는 선택이며, judge 판단은 [#218](https://github.com/woonyong-choi/saturn/issues/218)에서 잰다. |
| 구성 | 근거 사실이 모두 도구 결과 글에 있어 경로, 종료 코드, 수정 줄 수 같은 구조 값은 정답률에 거의 영향을 주지 않는다. | 변환 충실도 지표를 탐색 분석으로 함께 낸다. |
| 외적 | 합성 작업은 읽기와 명령 위주라 실제 작업의 도구 다양성이 없다. | 적용 범위를 가설 표에 적는다. |
| 외적 | 원시 줄이 1단계와 같아 provider 버전이 바뀐 뒤의 출력은 재지 않는다. | 버전을 `env.json`에 남기고 한계에 적는다. |
