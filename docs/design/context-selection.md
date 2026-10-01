# 맥락 고르기

| 항목 | 값 |
|---|---|
| 상태 | 결정 |
| 관련 결정 | [judge가 후보 전체를 판단하고 코드 순위는 대체 순서로만 쓴다](../decisions/2026-10-01-judge-all-candidates.md), [같은 뜻 찾기는 judge에 맡기고 용어 카탈로그를 두지 않는다](../decisions/2026-10-01-judge-decides-synonyms.md), [경쟁 구역은 기준값 없이 judge 남김 확률 순으로 예산까지 채운다](../decisions/2026-10-02-fill-packet-by-probability.md) |

## 요약

맥락 고르기는 패킷, 결과 전달, 파일 순위에 넣을 항목을 고르는 기능이다. `core`의 `sessions`가 후보마다 파일 겹침, 단어 겹침, 최근성 순위를 매기고 RRF로 합친다. judge는 후보 전체를 판단하고, 합친 순위는 judge가 답하지 못했을 때의 순서와 같은 확률일 때의 순서로만 쓴다. 같은 흐름에서 judge는 입력이 제약인지, 새 제약이 옛 제약을 대체하는지도 판단한다.

## 동기

긴 채팅에는 도구 호출과 다른 에이전트 결과가 수백 개 쌓인다. 후보를 코드 순위의 상위 N개로 좁혀 judge에 물으면 judge가 남길 항목을 놓친다. 후보 전체를 판단하게 한 측정에서 judge가 남긴 항목 중 RRF 상위 10개에 든 비율은 12.0%였고 무작위 기대값은 8.7%였다. 상위 40개도 44.5%였다([#116](https://github.com/woonyong-choi/saturn/issues/116) 측정). judge가 답하지 못하면 대체 규칙이 생략이나 경로만이라 확실히 관련 있는 항목까지 빠진다. 사용자가 뒤집은 제약이 옛 제약과 나란히 패킷에 들어가면 새 session은 어느 쪽을 따를지 모른다. 이 기능은 judge가 후보 전체를 판단하게 하고, 큰 요청은 질문 단위로 나눠 보내며, judge가 실패하면 강제한 전환만 순위로 고르고, 대체된 제약을 정리한다.

## 예시

### 도구 호출 150개에서 패킷의 도구 결과 고르기

1. 사용자가 Claude로 40턴 작업한 뒤 Codex로 바꾸고 `로그인 실패 메시지 고쳐 줘`를 보낸다.
2. `sessions`는 도구 호출 150개에 파일 겹침, 단어 겹침, 최근성 순위를 매기고 RRF로 합친다.
3. 12턴의 `/v2/auth` 응답은 `auth/` 경로와 `로그인` 단어가 겹쳐 상위에 든다.
4. engine은 150개 전체를 `compact` 질문으로 judge에 묻는다. 질문 300개와 state가 요청 한 건의 크기 한도를 넘으면 질문 단위로 나눠 보낸다.
5. `sessions`는 항목을 남김 확률이 높은 순으로, 같은 확률이면 순위 순으로 패킷의 경쟁 구역에 예산이 찰 때까지 채운다.

### judge가 답하지 않을 때

1. 사용자가 모델을 고정해 새 session으로 옮기는 중에 패킷의 `compact` 판단이 응답하지 않는다.
2. engine은 5초 간격으로 두 번 다시 보내고, 10초 뒤에도 실패하면 로그에 `판단 모델 실패로 기록 선택을 건너뜁니다`를 남긴다.
3. `sessions`는 전환하고 RRF 순서대로 경쟁 구역의 예산까지 채운다.
4. `/v2/auth` 응답은 순위 상위라 judge 판단 없이도 패킷에 들어간다.

### 제약이 뒤집힐 때

1. 사용자가 3턴에 `에러 메시지는 영어로 통일해`를 보낸다.
2. judge가 `is_constraint`에 0.9를 답하고, `sessions`는 이 입력 원문을 제약으로 등록한다.
3. 사용자가 25턴에 `아니, 한국어로 바꿔`를 보낸다.
4. judge가 두 원문을 나란히 놓은 `replaces_1`에 0.93을 답하고, `store`는 25턴 제약이 3턴 제약을 대체한다고 기록한다.
5. 다음 패킷에는 25턴 원문과 `3턴의 "에러 메시지는 영어로 통일해"를 대체`가 함께 들어간다.

## 상세 설계

### 고르기를 쓰는 곳

| 쓰는 곳 | 후보 | judge 질문 |
|---|---|---|
| 패킷의 경쟁 구역 | 이 채팅의 도구 호출과 결과 | `compact` |
| 기록 번호로 결과 전달 | 기록 번호 뒤 다른 에이전트의 결과 요약 | `compact` |
| 파일 순위 | 코드 검색이 찾은 파일 | `file-rank` |

- 문서 조각을 고르는 `context-select`는 이 흐름을 쓰지 않는다. 문서 조각은 `doc-filter`의 인젝션 판단을 거쳐야 하므로 judge 없이 순위만으로 넣지 않기 위해서다.
- `Reasoning` 종류의 도구 호출은 기록에 남기되 후보에서 뺀다. 추론은 도구 결과가 아니라 후보 수와 메모를 provider마다 다르게 만들기 때문이다.
- 패킷의 구역과 채우기 규칙은 [맥락 정리](context-management.md)에, 결과 전달은 [provider 연결과 session](providers-and-sessions.md)에 있다.

### 순위 채널

| 채널 | 쓰는 값 | 순위 |
|---|---|---|
| 파일 겹침 | 후보가 건드린 파일 경로. 도구 호출 이벤트의 경로 목록(`detail.paths`)에서 꺼낸다. | 기준 파일과 겹치는 경로 수가 많을수록 위 |
| 단어 겹침 | 후보의 글과 마지막 입력을 단어 조각으로 나눈 것 | BM25 점수가 높을수록 위 |
| 최근성 | 후보의 기록 번호 | 클수록 위 |

- 기준 파일은 마지막 입력에 나온 경로와 메인 session이 최근 3턴에 건드린 파일이다.
- 경로는 NFC로 정규화하고 앞의 `./`를 떼고 비교한다. 같은 파일이 표기 차이로 어긋나지 않게 하기 위해서다.
- 단어 겹침의 BM25는 `k1 = 1.2`, `b = 0.75`(초안)이고 마지막 입력의 조각은 중복 없이 한 번씩 센다.
- 한 채널 안에서 값이 같은 후보는 같은 순위다. 값이 같은데 순서만으로 점수가 갈리지 않게 하기 위해서다.
- 채널에 값이 없는 후보는 그 채널 순위에서 빠진다. 최근성은 모든 후보에 있다.
- 세 채널 모두 기록에 이미 있는 값을 계산만 한다. 라벨링이나 모델 호출이 없어 `core`가 파일, 네트워크, 프로세스를 다루지 않고 계산하기 위해서다.
- 셸 명령처럼 경로가 인자에 따로 없는 도구 호출은 경로 모양의 글자만 꺼낸다(초안).

### 단어 조각

1. 글을 유니코드 NFC로 정규화한다. macOS 파일 이름처럼 자모로 풀린 한글을 음절로 합쳐야 한글 구간으로 읽기 때문이다.
2. 글자마다 유니코드 범위로 종류를 정하고, 종류가 바뀌는 곳에서 나눈다. `로그인login`은 `로그인`과 `login`이 된다.
3. 종류마다 다음 표대로 조각을 만든다.

| 종류 | 예 | 조각 |
|---|---|---|
| 한글 | `로그인실패` | 글자 2개씩 겹치게: `로그`, `그인`, `인실`, `실패` |
| 라틴 문자(악센트 포함) | `authLogin.rs` | 소문자로 바꾸고 camelCase, snake_case, kebab-case 경계에서 나눈 단어: `auth`, `login`, `rs` |
| 숫자 | `404`, `v2` | 이어진 숫자 하나 |
| 한자, 가나 | `設定`, `ログイン` | 한글과 같이 글자 2개씩 |
| 기호 | `/`, `.`, `_`, `-`, `::` | 조각을 만들지 않고 나누는 경계로만 쓴다 |
| 공백, 이모지, 제어 문자 | | 버린다 |

- 한글을 글자 2개 단위로 자르는 것은 띄어쓰기 누락, 조사, 복합명사에서도 겹침을 찾기 위해서다. 한국어 검색에서 n-gram 색인은 사전 없이 복합명사를 다루고 형태소 기반 색인보다 효과가 좋았다(Lee & Ahn, SIGIR 1996; Lee, Cho, Park, Information Processing & Management 1999).
- 한자와 가나를 글자 2개 단위로 자르는 것은 중국어와 일본어도 띄어쓰기가 단어 경계가 아니기 때문이다. 버리면 그 언어 사용자는 단어 겹침 채널을 쓰지 못한다.
- 기호를 경계로만 쓰는 것은 `auth/login.rs`의 `/`와 `.`처럼 식별자 경계 역할을 하지만 그 자체로는 뜻이 없기 때문이다.
- 영문을 식별자 경계에서 나누는 것은 기록의 영문 대부분이 코드 식별자와 경로이기 때문이다.
- 한자와 가나는 한 종류로 본다. 일본어는 한자와 가나를 섞어 한 단어를 쓰기 때문이다.
- 한 글자뿐인 한글, 한자, 가나 구간은 그 글자 하나를 조각으로 둔다. 한 글자 단어를 버리지 않기 위해서다.
- 라틴 밖 알파벳 문자(키릴, 그리스 문자 등)는 라틴 문자와 같은 규칙으로 자르되 다른 종류로 본다.
- 오타는 오타 글자가 든 조각만 빠지므로 점수가 낮아질 뿐 0이 되지 않는다.
- 같은 뜻의 다른 말과 번역어는 judge가 후보 전체를 판단하면서 직접 판단한다. 용어 카탈로그와 임베딩 채널은 두지 않는다([결정 기록](../decisions/2026-10-01-judge-decides-synonyms.md)). 작은 다국어 모델 두 개로 잰 결과, 한국어 설명 질의의 정답 코드 묶음을 상위 10개에 올린 비율은 단어 기반과 같은 0.0%였고, 단어가 겹치는 질의의 상위 10개 재현율은 10.5%p 이상 낮아졌다([실험 보고서](../experiments/embedding-synonym/report.md)). 설치 크기 342.5MB 이상과 상주 메모리 287.4MB 이상도 든다.
- 한글을 자모 3개 단위로 자르지 않는다. 오타 질의 재현율 이득이 0.9%p [−0.8, 2.6]에 그치고 오타 없는 질의의 1위 정밀도가 10.0%p 떨어졌기 때문이다([실험 결과](../experiments/wordpiece-typo-recall/report.md)).
- 영문 식별자 단어를 글자 4개 단위로 바꾸지 않는다. 오타 질의 재현율은 11.6%p 올랐지만 오타 없는 질의의 1위 정밀도가 5.2%p 떨어졌기 때문이다([실험 결과](../experiments/wordpiece-typo-recall/report.md)). 영문 글자 n-gram의 근거는 McNamee & Mayfield, Information Retrieval 2004다.

### 순위 합치기

`sessions`는 채널 순위를 RRF(Reciprocal Rank Fusion)로 합친다. `r_c`는 채널 c에서 후보의 순위(1부터)이고, `k`는 합치기 상수다.

```text
점수 = Σ_c 1 / (k + r_c)
```

- 점수 대신 순위를 쓴다. 채널마다 값의 단위가 달라 그대로 더할 수 없기 때문이다.
- `k`의 기본값은 30이다. 실측에서 `k` 10, 30, 60, 100의 차이는 상위 40개 기준 1.0%p 이하였고, 사전 등록한 규칙대로 상위 40개 비율이 가장 높은 값을 골랐다([실험 결과](../experiments/rrf-k-top-n/report.md)).
- 원 논문은 여러 검색 결과를 합칠 때 `k`=60을 썼다(Cormack, Clarke, Büttcher, SIGIR 2009). `k`가 클수록 한 채널의 1등보다 여러 채널에 고르게 든 후보가 이긴다.
- 점수가 같으면 기록 번호가 큰 후보를 위에 둔다.
- `k`는 judge가 답하지 못했을 때의 순서와 같은 확률일 때의 순서에만 쓰이며 실측으로 정한다([#116](https://github.com/woonyong-choi/saturn/issues/116)).

### judge에 넘기기

1. 후보 전체를 judge에 묻는다. 후보 수로 줄이지 않는다.
2. 요청이 크기 한도를 넘으면 질문 단위로 나눠 여러 요청으로 병렬 전송하고, 조각마다 같은 state를 싣는다. 동시 수와 한도는 [judge 호출](judge.md#judge-호출)에 있다.
3. 항목의 남김 확률은 `call_<id>_keep`과 `result_<id>_keep` 중 큰 값이다. 하나만 답했으면 그 값이다.
4. 최종 순서는 답이 있는 항목을 남김 확률이 높은 순으로 두고, 같은 확률이면 RRF 순으로 둔다. 기준값은 없고 확률이 낮은 항목도 빼지 않는다.
5. judge가 답하지 못한 항목은 답이 있는 항목 뒤에 RRF 순으로 둔다. 실패한 조각의 항목도 같다.
6. judge가 전부 답하지 못하면 재시도가 끝난 뒤 판단 없이 진행한다. judge가 시작한 전환은 건너뛰고 현재 모델로 진행하며, 사용자가 고정했거나 맥락 크기 규칙이 시작한 전환은 RRF 순서로 경쟁 구역의 예산까지 채운다. 재시도와 로그는 [judge 실패](judge.md#judge-실패)에 있다.

- judge가 남길 항목을 순위로 미리 자르지 않기 위해서다. 후보 전체를 판단한 측정에서 남은 항목 중 RRF 상위 10개에 든 비율은 12.0% [9.2, 15.6]였고 무작위 기대값은 8.7%였다. 상위 40개도 44.5% [39.7, 49.4]였고, 95%에 닿으려면 후보 중앙값 127개보다 많은 132~133개가 필요했다([#116](https://github.com/woonyong-choi/saturn/issues/116) 측정).
- 전체를 묻는 비용은 낮다. 기준 judge의 입력 비용은 100만 토큰당 $0.042이고 같은 질문의 일치율은 98.4%였다([#116](https://github.com/woonyong-choi/saturn/issues/116) 측정).
- 기준값을 두지 않는다. 근거 항목의 남김 확률은 평균 0.372, 최댓값 0.65여서 기준값 0.5가 근거 항목 624개 중 549개(88.0%)를 버렸고, 기준값 없이 확률 순으로 채우면 필요한 근거가 모두 든 질문이 11.7%에서 65.6%가 됐다. 확률은 근거와 비근거를 잘 가르므로(AUC 0.940) 순서에만 쓴다([결정 기록](../decisions/2026-10-02-fill-packet-by-probability.md), [후속 분석](../experiments/handoff-packet-quality/report.md#후속-분석-원인)).
- RRF 순위는 judge 판단을 보조하는 값이다. judge가 답하지 못할 때의 순서와 같은 확률일 때의 순서만 정하므로, 순위가 낮아도 judge가 답하면 먼저 들어간다.
- 나누는 규칙은 `core`가 요청 목록을 만드는 순수 함수이고, 전송과 응답 모으기는 engine이 한다. `core`가 네트워크를 다루지 않기 위해서다.

### 도구 결과 메모

축약본은 결과 앞 300자와 한 줄 메모다. 메모는 도구 종류별 틀로 `sessions`가 만든다. 모델 호출 없이 같은 결과에서 늘 같은 메모를 만들기 위해서다.

| 도구 종류 | 메모 틀 |
|---|---|
| 셸 명령 | 명령 · 종료 코드 · 줄 수 · `error`, `failed`, `panic`이 든 첫 줄 |
| 테스트 실행 | 통과 수, 실패 수, 실패한 테스트 이름 |
| 파일 읽기 | 경로 · 읽은 줄 범위 |
| 파일 수정 | 경로 · 더한 줄 수와 지운 줄 수 |
| 웹 요청 | 주소 · 상태 코드 |
| 그 밖 | 메모 없음 |

engine의 `providers`가 provider 도구 이름을 Saturn 도구 종류로 바꾸고, 경로, 읽은 줄 범위, 바뀐 줄 수, 종료 코드를 이벤트에 싣는다. provider 고유 이름을 `providers/codex`, `providers/claude` 안에만 두기 위해서다. 이벤트 필드는 [provider 연결과 session](providers-and-sessions.md#이벤트-수신과-변환)에 있다.

### 제약 식별

1. engine은 `route` 질문 세트에 `is_constraint`를 함께 묻는다.
2. 답이 기준값 이상이면 `sessions`는 그 입력 원문을 제약으로 등록한다.
3. TUI는 등록한 입력에 제약 표시를 붙이고, 사용자는 등록을 취소할 수 있다.
4. 사용자가 취소하면 `sessions`는 등록을 지우고 judge 학습의 틀림 신호로 기록한다.

- `is_constraint`는 세 조건을 모두 채우는 입력을 제약으로 본다. 사용자가 한 말이다. 이번 요청 하나가 아니라 앞으로도 적용된다. 무엇을 할지가 아니라 언어, 도구, 형식, 금지처럼 어떻게 할지를 제한한다.
- 입력마다 이미 보내는 `route` 요청에 질문 하나를 더하므로 judge 호출 수는 늘지 않는다. judge는 state를 한 번 읽고 모든 질문에 답한다.
- 원문을 그대로 등록한다. 요약하지 않아 뜻이 바뀌지 않게 하기 위해서다.

### 제약 대체

1. 새 제약이 등록되면 `sessions`는 단어 조각이나 파일이 겹치는 기존 제약을 최대 10개 고른다.
2. engine은 `constraint` 질문 세트로 기존 제약마다 `replaces_<n>`을 묻는다. state에는 두 원문과 순서를 넣고, 질문은 `나중 제약을 따르면 앞 제약을 지킬 수 없다`이다.
3. 답이 0.8 이상이면 `store`는 새 제약이 앞 제약을 대체한다고 기록한다.
4. 답이 0.5 이상 0.8 미만이면 두 제약을 충돌 가능으로 기록한다.
5. 패킷에는 대체된 제약을 따로 넣지 않고, 새 제약 원문 뒤에 `{턴}턴의 "{앞 제약 원문}"를 대체`를 붙인다.
6. 충돌 가능한 두 제약은 둘 다 넣고 `충돌 가능` 표시를 붙인다.

- 두 원문을 나란히 놓고 직접 비교해 묻는다. judge는 앞 말을 가리키는 간접 지시와 여러 단계 추론에 약하기 때문이다.
- 대체해도 원문은 지우지 않는다. 기록 원문을 정본으로 두기 위해서다.
- 판단이 없거나 기준 밖이면 대체하지 않는다. 잘못 지운 제약은 새 session이 알 수 없기 때문이다.

### 오류 처리

| 상황 | 동작 |
|---|---|
| `compact`, `file-rank` judge 호출이 재시도 뒤에도 실패 | judge가 시작한 전환은 건너뛰고 현재 모델로 진행한다. 그 밖에는 RRF 순서로 경쟁 구역의 예산까지 채운다. |
| 요청 조각 일부 실패 | 실패한 조각의 항목은 남기고, 답이 있는 남긴 항목 뒤에 RRF 순으로 둔다. |
| 크기 한도를 넘는 `state` | 질문 하나도 담을 수 없으므로 요청을 만들지 않고 RRF 순서로 채운다. |
| `is_constraint` 판단 없음 | 제약으로 등록하지 않는다. |
| `replaces_<n>` 판단 없음 | 대체도 충돌 가능도 기록하지 않는다. |
| 도구 호출 인자에 경로 없음 | 파일 겹침 채널에서 그 후보를 뺀다. |

### 요구사항

| 요구사항 | 검증 계획 |
|---|---|
| 도구 결과 메모의 종료 코드, 경로, 줄 수를 provider와 무관하게 이벤트에서 얻는다. | [provider 연결과 session](providers-and-sessions.md#요구사항)의 도구 호출 값 행 |
| 후보 전체를 judge에 묻는다. | `saturn-terminal/core/src/judges/mod.rs`의 `compact_questions_150_candidates_ask_all` |
| 요청이 크기 한도를 넘으면 질문 단위로 나누고 조각마다 같은 state를 싣는다. | `saturn-terminal/core/src/judges/split.rs`의 `split_request_over_limit_splits_by_question_with_same_state`, `saturn-terminal/core/src/judges/mod.rs`의 `compact_requests_large_state_splits_and_every_piece_carries_state` |
| 최종 순서는 답이 있는 항목의 남김 확률 순이고 같은 확률이면 RRF 순이며 확률이 낮은 항목도 빼지 않는다. | `saturn-terminal/core/src/sessions/ranking.rs`의 `order_after_judge_orders_by_probability_and_keeps_low`, `order_after_judge_same_probability_follows_rrf_order` |
| 항목의 남김 확률은 호출과 결과 중 큰 값이다. | `saturn-terminal/core/src/judges/mod.rs`의 `compact_verdicts_takes_larger_of_call_and_result` |
| 답이 없는 항목(실패한 조각 포함)은 RRF 순으로 뒤에 둔다. | `saturn-terminal/core/src/sessions/ranking.rs`의 `order_after_judge_unanswered_follow_answered_in_rrf_order`, `saturn-terminal/core/src/judges/mod.rs`의 `compact_verdicts_merges_pieces_and_skips_failed_piece` |
| judge가 전부 답하지 못하면 RRF 순서로 예산까지 채운다. | `saturn-terminal/core/src/sessions/ranking.rs`의 `order_after_judge_no_verdicts_keeps_rrf_order`, `saturn-terminal/core/src/sessions/packet.rs`의 `build_packet_judge_no_response_fills_in_rrf_order` |
| 띄어쓰기와 조사가 달라도 같은 한글 조각을 만든다. | `saturn-terminal/core/src/sessions/fragments.rs`의 `fragments_spacing_and_particle_share_hangul_bigrams` |
| 영문 식별자는 식별자 경계에서 나눈다. | `saturn-terminal/core/src/sessions/fragments.rs`의 `fragments_identifier_splits_at_case_and_symbols` |
| 자모로 풀린 한글도 음절 한글과 같은 조각을 만든다. | `saturn-terminal/core/src/sessions/fragments.rs`의 `fragments_nfd_hangul_matches_nfc` |
| 같은 결과에서 늘 같은 메모를 만든다. | `saturn-terminal/core/src/sessions/memo.rs`의 `tool_memo_same_result_gives_same_memo` |
| 대체된 제약 원문은 기록에 남고 패킷에서만 빠진다. | 대체 뒤 기록에 두 원문이 있고 패킷에는 새 원문과 대체 표시만 있는지 확인한다. |
| 순위가 judge 전체 판단과 얼마나 겹치는지 잰다. | [RRF k와 judge 상위 N 실험 결과](../experiments/rrf-k-top-n/report.md): 상위 10개 12.0%, 상위 40개 44.5% |
| 단어 조각 단위가 오타 입력에서 관련 후보를 놓치지 않는다. | [단어 조각 단위별 오타 재현율 실험 결과](../experiments/wordpiece-typo-recall/report.md) |
| `is_constraint`와 `replaces_<n>`이 한국어 입력에서 기준 정확도를 넘는다. | [제약 식별과 대체 판정 정확도](../experiments/constraint-judge-accuracy/report.md)에서 `is_constraint` 0.7은 확인했다. [간접 지시 정확도](../experiments/indirect-constraint-accuracy/report.md)에서 `replaces_<n>` 구간과 간접 지시 입력은 기준을 가르지 못해 보류이고(간접 지시 입력 74.5% [68.0, 80.0]), 앞 입력의 제약 등록 여부를 state에 넣어도 정확도는 오르지 않았다. |
| judge 없이 순위로 채운 패킷은 judge 전체 판단 패킷보다 정답률이 10%p를 넘게 낮지 않다. | [새 패킷 규칙의 전환 품질 재측정](../experiments/handoff-packet-quality-v2/report.md): 정답률 차이 +50.7%p [44.7, 56.7]로 기각. 판단 없는 패킷의 규칙은 [judge 실패](judge.md#judge-실패)에 있다. |
| judge가 시작한 전환은 `compact` 판단이 실패하면 건너뛰고, 강제한 전환은 순위 순서로 채운다. | `saturn-terminal/core/src/judges/failure.rs`의 `compact_failure_skips_judge_transition_and_fills_forced_one`, `compact_failure_forced_transition_orders_competing_zone_by_rank` |

## 단점

- judge가 실패해 순위 순서로 채운 패킷은 정답률이 20.6%로 패킷 없음(20.0%)과 같은 수준이다([재측정 결과](../experiments/handoff-packet-quality-v2/report.md)). 그래서 judge가 시작한 전환은 건너뛰고, 강제한 전환만 이 패킷으로 채운다.
- 순위 채널은 같은 뜻의 다른 말을 모르므로 judge가 모두 실패하면 대체 순서에서 같은 뜻의 후보를 놓칠 수 있다.
- RRF 상위 40개는 judge 전체 판단이 남긴 항목의 절반 이상을 놓친다([실험 결과](../experiments/rrf-k-top-n/report.md)).
- 영문 단어 사이 공백이 빠지면 소문자 단어가 하나로 붙어 단어 겹침을 놓친다. 이 오타의 상위 10개 재현율은 56.4%였다([실험 결과](../experiments/wordpiece-typo-recall/report.md)).
- 후보가 150개면 질문이 300개라 judge 입력 토큰이 후보 수에 비례한다. 큰 요청은 여러 건으로 나뉘고, 병렬로 보내면 지연은 조각 수와 거의 무관하게 약 0.3초다([#179](https://github.com/woonyong-choi/saturn/issues/179) 실측).
- 나뉜 요청은 조각마다 같은 state를 보내 입력 토큰이 조각 수만큼 늘고, 조각이 실패하면 그 항목은 순위로만 정해진다.
- 동시 요청이 8개를 넘을 때의 속도 제한은 재지 않았다.
- `k`, 기준 파일 범위를 실측으로 맞춰야 한다.
- 일과 제약이 섞인 입력은 원문 전체가 제약으로 등록되어 패킷이 길어진다.
- 앞 말을 가리키는 간접 지시 입력은 `is_constraint` 정확도가 74.5% [68.0, 80.0]이고, 앞 입력의 제약 등록 여부를 state에 넣어도 오르지 않았다([간접 지시 정확도](../experiments/indirect-constraint-accuracy/report.md)).

## 대안

- RRF 상위 N개만 judge에 묻는 방식은 judge가 남길 항목을 상위 10개에서 12.0%, 상위 40개에서도 44.5%만 담아 버렸다([결정 기록](../decisions/2026-10-01-judge-all-candidates.md)).
- 채널 점수의 가중합은 단위가 다른 점수의 가중치를 따로 학습해야 해 버렸다([결정 기록](../decisions/2026-10-01-ranked-candidates-before-judge.md)).
- 남김 확률 0.5 이상만 경쟁 구역에 넣는 방식은 근거 항목의 88.0%를 버려 버렸다([결정 기록](../decisions/2026-10-02-fill-packet-by-probability.md)).
- 임베딩 채널을 기본으로 넣는 방식은 같은 뜻 질의에서 이득이 없고 단어가 겹치는 질의의 재현율과 설치 크기, 상주 메모리를 잃어 버렸다([결정 기록](../decisions/2026-10-01-judge-decides-synonyms.md), [실험 보고서](../experiments/embedding-synonym/report.md)).
- 입력마다 LLM으로 사실 문장을 뽑는 방식은 호출과 출력 비용이 들고 원문 대신 생성문을 저장해 버렸다.

## 미해결 질문

- judge에 넘길 후보를 RRF 상위 N으로 고를지, 후보 전체나 다른 거르기로 바꿀지([#153](https://github.com/woonyong-choi/saturn/issues/153))
- 간접 지시 입력에서 `is_constraint` 정확도를 올리되 일반 제약 입력의 재현율을 해치지 않는 질문 문장과 부분 충돌을 따로 묻는 질문이 있는지([#186](https://github.com/woonyong-choi/saturn/issues/186))
