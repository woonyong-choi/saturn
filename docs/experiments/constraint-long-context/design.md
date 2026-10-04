# 실제 대화 기록의 제약 판단 품질: 실험 설계

| 항목 | 값 |
|---|---|
| 이슈 | [#316](https://github.com/woonyong-choi/saturn/issues/316) |
| 관련 설계 | [맥락 고르기](../../design/context-selection.md), [router](../../design/router.md) |
| 사전 데이터 | 없음. 원본 Claude Code 기록은 수집 전 열람하지 않고 설계 커밋 뒤에 추출한다. |

## 질문

매우 긴 실제 Claude Code 대화에서 Jev의 `is_constraint` 등록 판단, 뒤집기·부분 변경의 `replaces_<n>` 판단, 제약이 많이 쌓였을 때 보존할 제약의 선택 품질을 잰다. `context-selection.md`의 기존 제약 10개 선택 규칙과 전체 유효 제약을 함께 주는 조건을 비교하고, 확신도와 제약 사이 거리로 오류가 달라지는지 확인한다.

## 가설

| 가설 | 예측 | 적용 범위 |
|---|---|---|
| H1 | 합의 라벨 기준 `is_constraint` 정밀도는 80%를 넘는다. | 실제 Claude Code 대화의 30·120·400 이상 사용자 턴 구간, `route@1.1` |
| H2 | 합의 라벨 기준 `is_constraint` 재현율은 80%를 넘는다. | H1과 동일 |
| H3 | 조건 A보다 조건 B의 제약 대체·부분 변경 판정 정확도가 5%p 이상 높다. | 합의된 제약 쌍, `constraint@1.0` |
| H4 | 대화 끝 유효 제약 집합을 고르는 정확도는 80%를 넘는다. | 유효 제약이 11개 이상인 표본의 조건 A·B |
| H5 | Jev 확신 구간이 높을수록 `is_constraint`와 제약 전이의 정확도가 높아진다. | 0.5 미만, 0.5 이상 0.7 미만, 0.7 이상 0.8 미만, 0.8 이상 |
| H6 | 제약 사이의 턴 거리가 짧을수록 뒤집기·부분 변경 판정 정확도가 높다. | 1~5, 6~20, 21~100, 101 이상 턴 |

## 설계

| 항목 | 값 |
|---|---|
| 조건 | A는 현재 입력, 설계 state, 단어·파일이 겹치는 기존 유효 제약 최대 10개를 보낸다. B는 현재 입력, 설계 state, 기존 유효 제약 전체를 보낸다. 두 조건 모두 같은 `route@1.1`과 `constraint@1.0` 질문을 사용한다. |
| 배정 | 대화와 표본 턴은 `random.Random(316)`으로 고정한다. 조건 순서는 표본마다 섞고, 같은 조건의 세 반복은 같은 요청을 연속으로 보낸다. |
| 눈가림 | Jev 수집기는 라벨과 정답을 읽지 않는다. 라벨러는 다른 라벨러의 답과 Jev 응답을 보지 않는다. 보고서에는 대화 원문, 턴 원문, 식별 가능한 경로를 넣지 않는다. |
| 환경 | macOS, Python 3.10 이상 표준 라이브러리, Jev `POST https://api.typesafe.ai/v1/systemone`, 요청 모델 `jev-1.13.0`, Codex 라벨 모델 `gpt-6-astra`, Claude 라벨 명령 `claude -p --model sonnet --tools ''`. Jev와 라벨러의 샘플링은 고정할 수 없으므로 반복과 원문 응답을 기록한다. |

### 질문과 state

질문 문장은 [questions.json](questions.json)에 고정한다. 질문의 문장은 앞선 `constraint-judge-accuracy`와 `indirect-constraint-accuracy` 실험에서 사용한 문장을 재사용한다.

| 질문 | 질문 세트 | state | 문장 |
|---|---|---|---|
| `is_constraint` | `route@1.1` | `previous_user_input`, `latest_user_input`, `prior_constraint_ids`, `active_constraint_count` | `The user's latest input sets a rule that applies beyond this single request and limits how the work is done (language, tool, format, or prohibition) rather than what to do.` |
| `replaces_<n>` | `constraint@1.0` | `earlier_constraint`, `later_constraint`, `turn_distance` | `Following the later constraint makes it impossible to keep the earlier constraint.` |
| `keep_<n>` | `context-select@1.0-long-context` | `latest_user_input`, all candidate constraints, `active_constraint_count` | `This constraint should remain active for the user's work at the end of this conversation.` |

`is_constraint`는 기존 설계의 세 조건을 모두 채울 때 1로 해석한다. 제약은 사용자 말이고, 이번 요청 하나가 아니라 앞으로도 적용되며, 무엇을 할지가 아니라 언어·도구·형식·이름·길이·금지·확인 절차처럼 방식을 제한한다. `replaces_<n>`은 0.8 이상을 대체, 0.5 이상 0.8 미만을 부분 변경 또는 충돌 가능, 0.5 미만을 양립으로 해석한다. `keep_<n>`은 0.7 이상인 항목을 남김으로 해석한다. `keep_<n>`은 공개 설계에 아직 없는 측정 전 질문이며, 이 실험에서만 사용하고 구현 계약으로 승격하지 않는다.

state에는 현재 대화의 해시 식별자, 현재 턴 식별자, 현재와 직전 사용자 입력, 제약 식별자·원문·턴 번호·파일 목록만 넣는다. 도구 결과와 다른 대화 원문은 넣지 않는다. 실제 요청 직전에 `router.md` 규칙대로 비밀값과 이메일을 가리고 절대 경로를 `[abs]/이름`으로 바꾼다.

### 표본 추출

`~/.claude/projects/*/*.jsonl`에서 `type`이 `user`인 사람이 쓴 글만 시간순으로 추출한다. 도구 결과, system 주입, `<system-reminder>`, `<command-*>`, `<local-command-*>`로 시작하거나 포함된 자동 삽입은 제외한다. 사용자 턴이 30개 미만인 파일은 후보에서 제외한다.

대화 전체 사용자 턴 수를 기준으로 30~119, 120~399, 400 이상 구간을 만든다. 각 구간에서 최대 2개를 경로 해시 순으로 고른다. `workspace/oss/saturn`을 포함하는 대화는 해당 구간에서 우선하고, 같은 대화는 한 번만 고른다. 각 대화에서는 첫 5개, 마지막 5개, 중간을 균등하게 나눈 턴을 포함해 최대 20개를 Jev 표본으로 고른다. 라벨러는 선택한 대화의 전체 사용자 턴을 읽는다.

라벨 합의가 된 표본 턴마다 앞선 합의 제약을 상태로 만든다. 조건 A의 후보는 `context-selection.md`와 같은 한글 bigram·식별자 토큰과 파일 겹침을 계산해 최대 10개를 고른다. 조건 B는 앞선 합의 제약 전체를 보낸다. 현재 표본 턴에 앞선 유효 제약이 없으면 `replaces_<n>` 질문은 만들지 않는다. 대화 끝의 유효 제약이 11개 이상이면 조건별로 세 반복의 `keep_<n>` 요청을 추가한다.

### 라벨 지시문

두 라벨러는 같은 지시문으로 각 대화 전체를 읽고 다음 JSON을 반환한다. 라벨러 호출은 대화 하나당 모델별 한 번이다.

```json
{
  "conversation_id": "...",
  "turns": [
    {
      "turn_id": "u-0001",
      "is_constraint": true,
      "operation": "register|replace|partial|release|none",
      "targets": ["u-0000"],
      "scope": "짧은 적용 범위",
      "reason": "세 조건을 판정한 짧은 근거"
    }
  ],
  "final_active_turn_ids": ["u-0001"]
}
```

`is_constraint`는 세 조건 표를 그대로 적용한다. `replace`는 같은 대상의 값이 바뀌어 앞 제약을 지킬 수 없는 경우다. `partial`은 앞 제약의 일부 범위만 예외가 되는 경우다. `release`는 앞 제약을 더 이상 적용하지 않도록 푸는 경우다. 두 라벨러의 `is_constraint`, 전이 종류, 대상, 마지막 유효 집합이 모두 같은 항목만 확인 분석에 쓴다. 불일치율은 전체 항목과 길이 구간별로 따로 보고한다. 불일치 항목 중 최대 20개를 `random.Random(316)`으로 뽑아 사용자 검토용 `.local` 목록에 쓴다.

## 변수

| 변수 | 종류 | 정의 | 단위 |
|---|---|---|---|
| `conversation_id` | 조작 | 비공개 원본 파일의 SHA-256 앞 16자리 | 없음 |
| `length_band` | 조작 | 전체 사용자 턴 수에 따른 `30-119`, `120-399`, `400+` | 구간 |
| `turn_id` | 조작 | 추출된 사용자 턴의 시간순 식별자 | 없음 |
| `condition` | 조작 | `A_top10_overlap`, `B_all_active` | 없음 |
| `repeat` | 조작 | 같은 요청의 1, 2, 3회 반복 | 회 |
| `is_constraint_answer` | 측정 | Jev `is_constraint`의 `noul` 확률 | 확률 |
| `replace_answer` | 측정 | Jev `replaces_<n>`의 `noul` 확률 | 확률 |
| `keep_answer` | 측정 | Jev `keep_<n>`의 `noul` 확률 | 확률 |
| `consensus_is_constraint` | 측정 | 두 라벨러가 일치한 제약 여부 | 0 또는 1 |
| `consensus_operation` | 측정 | 두 라벨러가 일치한 전이 종류 | 범주 |
| `consensus_final_set` | 측정 | 두 라벨러가 일치한 대화 끝 유효 제약 식별자 집합 | 집합 |
| `registration_log_accuracy` | 파생 | Jev 확률 기준으로 만든 한 줄 등록 문구의 동작·턴 식별자 일치율 | 비율 |
| `transition_accuracy` | 파생 | 대체·부분 변경·양립 판단과 정답 전이의 일치율 | 비율 |
| `final_set_accuracy` | 파생 | Jev가 남긴 집합과 합의 유효 집합의 exact match 비율 및 Jaccard 평균 | 비율 |
| `confidence_band_accuracy` | 파생 | 확률 구간별 정답 비율 | 비율 |
| `distance_band_accuracy` | 파생 | 두 제약의 턴 거리 구간별 전이 정답 비율 | 비율 |
| `latency_ms` | 측정 | Jev 요청 전송부터 응답까지의 시간 | ms |

## 표본

| 항목 | 값 |
|---|---|
| 출처 | 사용자의 로컬 `~/.claude/projects/*/*.jsonl` 원본. 원본은 읽기만 한다. |
| 크기 | 대화 최대 6개, 길이 구간별 최대 2개, 대화별 Jev 표본 최대 20턴, Jev 표본 최대 120턴 |
| 크기 근거 | 예산. 등록·전이 요청은 표본 120턴 × 조건 2 × 반복 3 = 720회다. 대화별 보존 요청은 조건 2 × 반복 3 × 최대 6개 = 36회이며, 유효 제약이 11개 이상인 대화만 실제 요청을 만든다. 최대 총 756회로 Jev 1,500회보다 작다. 라벨은 모델별 최대 6회로 각 40회보다 작다. |
| 중단 규칙 | Jev 호출 수가 1,500회에 도달하면 다음 호출을 하지 않고 중단한다. 어느 라벨러든 40회에 도달하면 다음 호출을 하지 않고 중단한다. 401·403 응답은 키를 폐기하지 않고 해당 실행을 멈추며, 키 문자열은 출력하지 않는다. |
| 반복과 예열 | Jev 판단마다 같은 요청을 3회 묻고 예열은 하지 않는다. 라벨러는 대화마다 한 번 호출하며 반복하지 않는다. |

## 분석

| 가설 | 지표 | 방법 | 채택 기준 |
|---|---|---|---|
| H1 | 길이별 `is_constraint` 정밀도 | 합의 항목의 TP/예측 양성 비율과 95% Wilson 신뢰구간 | 전체 및 각 길이 구간에서 하한이 80%보다 높으면 채택, 상한이 80%보다 낮으면 기각, 그 밖에는 보류 |
| H2 | 길이별 `is_constraint` 재현율 | 합의 양성 중 TP 비율과 95% Wilson 신뢰구간 | H1과 같은 규칙 |
| H3 | 조건별 전이 정확도 차이 | 2×2 paired 표, McNemar 검정 또는 불일치 25개 미만이면 정확 이항 검정, 조건 B-A 차이와 95% 신뢰구간 | 차이의 95% 신뢰구간 하한이 5%p보다 높으면 채택, 상한이 0%p보다 낮으면 기각, 그 밖에는 보류 |
| H4 | 조건별 최종 집합 exact match와 Jaccard | exact match 비율과 Jaccard 평균, 각각 95% Wilson 또는 bootstrap percentile 구간 | exact match 하한이 80%보다 높으면 채택, 상한이 80%보다 낮으면 기각, 그 밖에는 보류 |
| H5 | 확신 구간별 정확도 | 사전 등록한 네 확신 구간의 비율과 Wilson 구간, 구간 순서의 단조성은 탐색으로 분리 | 각 구간을 별도 보고하며 확정 판정은 하지 않는다 |
| H6 | 거리 구간별 정확도 | 사전 등록한 네 거리 구간의 비율과 Wilson 구간 | 각 구간을 별도 보고하며 확정 판정은 하지 않는다 |

등록·해제·대체 로그 후보 문구는 스크립트가 Jev의 판단에 따라 `제약 등록: {턴}`, `제약 해제: {턴}`, `제약 대체: {새 턴} -> {옛 턴}` 형식으로 만든다. 정확도는 필요한 동작과 모든 턴 식별자가 합의 라벨과 일치하는 비율로 계산한다. 문구의 자연스러움은 측정하지 않는다.

| 항목 | 규칙 |
|---|---|
| 제외 기준 | 자동 삽입 턴, 가림 검사 실패 요청, JSON 형식이 깨진 라벨, 두 라벨러가 불일치한 확인 분석 항목을 해당 지표에서 제외하고 제외 수를 보고한다. Jev의 `invalid`, `no_answer`는 오답이 아니라 별도 상태로 보고하고 임계값 판정에서는 행동하지 않은 것으로 처리한다. |
| 실패한 실행 | 네트워크 전송 전 실패만 설계의 재시도 규칙으로 최대 3회 재전송한다. 응답을 받은 뒤의 오류와 401·403은 재전송하지 않고 실패 행을 남긴다. 중단 전까지 받은 행은 분석에 포함한다. |
| 다중 비교 | H1과 H2의 길이 구간별 확인은 Holm-Bonferroni를 적용한다. H3와 H4는 각각 하나의 확인 비교로 본다. H5와 H6은 탐색 보고만 한다. |
| 재현성 | 표본·후보 순위·호출 순서는 시드 316으로 재현한다. Jev와 라벨러의 확률 응답은 재현 대상이 아니며 원문 응답과 모델명을 private 원자료에 보존한다. |

## 판정

| 결과 | 설계에 반영 |
|---|---|
| H1·H2 채택 | `is_constraint` 기준값을 유지하고 길이 구간별 오류 유형을 후속 설계에 반영한다. |
| H3 채택 | 전체 유효 제약을 함께 보내는 조건 B를 후속 검토 대상으로 삼는다. |
| H4 채택 | `keep_<n>` 질문과 집합 보존 규칙을 별도 설계 이슈로 제안한다. |
| H1~H4 기각 또는 보류 | 자동 등록·해제·대체를 확정하지 않고 낮은 확신에서 사용자 확인을 유지한다. |

## 탐색 분석

- 같은 요청의 세 반복 사이 Jev 확률 표준편차와 분류 일치율을 보고한다.
- 등록·해제·대체 로그 후보 문구의 누락 동작과 잘못된 턴 식별자를 분리해 센다.
- 대화별 원문 길이와 Jev latency의 분포를 보고한다.

## 타당성 위협

| 종류 | 위협 | 대응 |
|---|---|---|
| 내적 | 실제 기록의 라벨 정답이 없어 라벨러 합의를 정답으로 쓴다. | 라벨러를 독립 실행하고 불일치율과 무작위 20개 사용자 검토 목록을 남긴다. |
| 내적 | 같은 대화를 세 반복에 사용해 반복 간 상관이 생긴다. | 반복은 모델 안정성 측정으로만 쓰고 신뢰구간의 독립 단위는 표본 턴으로 둔다. |
| 구성 | 자동 삽입을 사람의 제약으로 잘못 읽을 수 있다. | JSONL의 `type`과 자동 삽입 표식을 함께 확인하고 가림 테스트를 통과한 행만 사용한다. |
| 구성 | 조건 A의 단어·파일 겹침 구현이 실제 `core`와 달라질 수 있다. | `context-selection.md`의 NFC, 한글 bigram, 식별자 분할, 파일 경로 규칙을 스크립트에 고정하고 후보 목록을 저장한다. |
| 외적 | 표본은 사용자의 Claude Code 기록과 현재 프로젝트에 치우친다. | 길이 구간과 Saturn 대화 포함을 사전 등록하고 결과의 적용 범위를 그 표본으로 제한한다. |
| 외적 | 라벨러와 Jev 모델 버전이 바뀔 수 있다. | 호출 응답의 모델명, 실행 날짜, 커밋, 환경을 `env.json`과 private 원자료에 기록한다. |

## 재현

```sh
./run.sh verify
./run.sh analyze
```

원본·추출 턴·라벨·Jev 응답은 공개 저장소에 저장하지 않는다. 모든 private 자료는 메인 저장소의 Git 제외 실행 디렉터리에만 저장하고, 공개 결과에는 집계 수치와 표만 둔다.

## 결론

| 가설 | 판정 | 반영한 문서 |
|---|---|---|
| H1 | 측정 전 | [맥락 고르기](../../design/context-selection.md) |
| H2 | 측정 전 | [맥락 고르기](../../design/context-selection.md) |
| H3 | 측정 전 | [맥락 고르기](../../design/context-selection.md) |
| H4 | 측정 전 | [맥락 고르기](../../design/context-selection.md) |
| H5 | 측정 전 | 없음 |
| H6 | 측정 전 | 없음 |
