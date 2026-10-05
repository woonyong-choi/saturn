# 모델 평가 근거 목록

| 항목 | 값 |
|---|---|
| 상태 | 구현됨 (#545) |
| 관련 설계 | [router](router.md#정책-고정), [모델 고르기](providers-and-sessions.md#모델-고르기), [설정](settings.md) |

## 요약

모델 평가 근거 목록은 모델마다 외부 벤치마크, 공개 사용 평가, 공식 지원 정보, Saturn 자체 실험을 출처와 수집일을 붙여 모아 둔 자료다. 목록은 검토를 거쳐 버전으로 배포하고, engine은 시작할 때 한 번 읽어 끝까지 쓴다. 이 자료로 후보마다 "품질을 확정했는가"를 가르고, 사용자 고정과 선호와 한도를 합쳐 router에 물을 후보 집합을 만든다. 순위나 여론만으로 모델을 승격하지 않는다.

## 근거 종류

| 종류 | 예 | 품질 확정 |
|---|---|---|
| `benchmark` | 외부 벤치마크 점수 | 날짜, 버전, 점수가 모두 있을 때 |
| `own_experiment` | Saturn 실험 결과 | 날짜, 버전, 점수가 모두 있을 때 |
| `official_spec` | provider 문서의 모델 ID, 추론 깊이, 한도, 가격 | 안 함. 지원 여부와 가격만 안다 |
| `public_opinion` | 공개 사용 평가, 여론 | 안 함. 발견과 재검증 순서의 참고다 |

- 모든 근거는 출처 주소와 수집일(`YYYY-MM-DD`)을 가진다. 없으면 배포 전 검사(`validate`)가 거른다.
- 항목은 `model_id`, 별칭, 고정 revision, 지원 추론 깊이, 기능, 가격(단위 포함), 불확실성, 근거 목록을 가진다. 벤치마크는 이름, 점수, 표본, 실행 환경을, 여론은 반례를 적는다.
- 외부 종합 순위를 Saturn의 성공률로 쓰지 않는다. 점수는 자료의 점수일 뿐 이 사용자의 작업 성과가 아니다.

## 품질 확정

`quality`는 모델 이름(id나 별칭), provider가 알려 준 지금의 revision, 오늘 날짜로 확정 여부를 낸다. 확정하지 못하면 이유를 함께 낸다.

| 이유 | 조건 |
|---|---|
| 목록에 없음 | 새 모델이다 |
| 목록 만료 | 오늘이 목록의 만료일 뒤다 |
| 근거 부족 | 항목의 revision을 잰, 날짜·버전·점수가 있는 벤치마크나 자체 실험이 없다 |
| revision 불명 | 항목의 revision이나 지금 모델의 revision을 모른다 |
| revision 바뀜 | 지금 revision이 근거를 잰 revision과 다르다. 별칭이 새 모델을 가리키는 경우다 |

별칭이 바뀐 모델의 이전 성능은 새 모델의 실측으로 쓰지 않는다. 근거는 잰 revision(`model_revision`)과 항목 revision이 같을 때만 그 모델의 근거다.

## 후보 집합

`candidate_set`은 provider가 지금 알린 모델, provider별 한도, 사용자 고정, 사용자 선호로 후보를 만든다.

- 사용자가 `/model`로 명시 고정한 모델이 있으면 그 모델이 후보를 대신한다. 후보는 비고 router는 `target_model`을 묻지 않는다. 품질 자료, 한도, 선호가 고정을 덮지 않는다.
- 한도를 다 쓴 provider의 모델은 후보에서 뺀다. 한도를 알 수 없는 provider는 후보에 두고 한도 미상으로 표시한다.
- 선호는 한도가 남은 품질 확정 후보의 순서만 앞당긴다. 선호한 모델이 지금 알려지지 않았거나, 한도를 다 썼거나, 품질을 확정하지 못했으면 후보 순서를 바꾸지 않고 건너뛴 이유를 남긴다. 선호는 강제 고정이 아니다.
- 품질을 확정하지 못한 후보도 후보에 남고 미검증으로 표시한다. 어느 후보를 쓸지는 고르는 쪽이 정한다.
- 후보가 없으면 빈 집합이고, 고르는 쪽은 [기본 모델과 선택 방식](providers-and-sessions.md#기본-모델과-선택-방식)의 대체 순서를 따른다.

## 배포와 정책 버전

- 목록은 `saturn-terminal/engine/data/model-catalog.json`에 두고 실행 파일에 담는다. 기록 저장소에 같은 내용을 따로 저장하지 않는다. 한도 같은 가용성은 실행 때 provider에서 받는다.
- 운영 중에 웹 검색으로 기준값을 바꾸거나 새 모델을 자동으로 켜지 않는다. 새 자료는 검토와 회귀 평가를 거쳐 새 버전으로 배포한다.
- 목록의 버전은 [정책 지문](router.md#정책-고정)에 들어간다. 목록은 engine 시작 때 정해지므로 한 실행 안에서는 바뀌지 않고, 새 버전 배포와 옛 버전으로의 되돌리기(rollback)는 engine 재시작으로만 일어난다. 지문이 다르면 다른 정책이다.
- 목록을 읽지 못하거나 검사에 실패하면 빈 목록을 쓰고 모든 모델을 미검증으로 본다.

## 현재 목록

버전 `2026-10-06.1`, 수집일 2026-10-06, 만료일 2026-12-31. 항목은 공식 지원 정보만 있다.

| provider | 모델 | 출처 |
|---|---|---|
| claude | `claude-fable-5-1`, `claude-opus-5-5`(별칭 `opus`), `claude-sonnet-5-5`(별칭 `sonnet`), `claude-haiku-4-5`(별칭 `haiku`) | [Claude models overview](https://platform.claude.com/docs/en/about-claude/models/overview) |
| codex | `gpt-6-astra`, `gpt-6.1-sol`, `gpt-6-luna` | [OpenAI models](https://developers.openai.com/api/docs/models) |

- 벤치마크와 자체 실험 점수는 아직 넣지 않았다. 그래서 지금은 모든 모델이 미검증이고, 선호로 순서를 앞당기는 일도 없다. 점수는 출처와 버전, 잰 revision을 확인한 뒤 새 목록 버전으로 더한다.
- Claude의 `revision`은 공식 문서가 모델 ID를 고정 스냅샷이라고 밝힌 값이다. Codex는 문서가 고정 revision을 밝히지 않아 비워 두었고 revision 불명으로 본다.
- provider가 실제로 알려 주는 모델 이름이 목록의 id나 별칭과 다르면 목록에 없는 모델로 본다.

## 아직 연결하지 않은 것

후보 집합 함수와 품질 확정은 `saturn-core`의 규칙과 시험까지다. router 요청의 `target_model` 후보, 선호 설정 키, 한도 조회는 아직 이 규칙에 연결하지 않았다. 그림자 판단은 [#527](https://github.com/woonyong-choi/saturn/issues/527), 적용은 [#531](https://github.com/woonyong-choi/saturn/issues/531)에서 연결한다.

## 요구사항

| 요구사항 | 시험 |
|---|---|
| 날짜·버전 없는 점수, 여론, 공식 사양, 다른 revision의 근거는 품질을 확정하지 않는다. | `saturn-terminal/core/src/models/tests.rs`의 `only_dated_versioned_measurements_of_the_same_revision_confirm_quality` |
| 출처나 날짜가 없거나 중복인 목록은 배포 전 검사가 거른다. | `validation_rejects_missing_sources_bad_dates_and_duplicates` |
| Claude만, Codex만, 양쪽, 후보 없음, 한도 미상, 한도 소진, 명시 고정에서 후보와 선호 처리가 맞다. | `candidate_sets_follow_availability_limits_pins_and_preferences` |
| 배포한 목록이 검사를 통과하고 점수 없는 모델은 미검증이다. | `saturn-terminal/engine/src/model_catalog.rs`의 `shipped_catalog_is_valid_and_confirms_no_quality_without_measurements` |
| 목록 버전을 바꾸면 정책 지문이 바뀌고 되돌리면 같다. | `saturn-terminal/engine/src/lifecycle/policy.rs`의 `inputs_keep_the_policy_they_were_accepted_under_across_swap_rollback_and_restart` |
