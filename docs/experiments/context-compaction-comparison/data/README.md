# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | 기존 합성 시나리오와 패킷을 고정하고 원본 fast-jev 라이브러리, Jev, Claude Sonnet을 호출했다. |
| 수집 기간 | 2026-10-05 |
| 개수 | 합성 시나리오 48개, 조건별 Sonnet 답 48개, Jev 선별 48개 |
| 표본 여부 | 기존 합성 자료 48개 전수 |
| 라벨 | 기존 합성 생성기가 질문별 정답 값과 근거 항목을 부여했다. 사용자 확인 정답이 아니다. |
| 알려진 문제 | 4세션을 하나의 라이브러리 입력으로 폈고 provider hook은 실행하지 않았다. |
| 개인정보 | 이번 합성 시나리오에는 개인 자료가 없다. API 키는 원자료에 저장하지 않았다. |
| 라이선스 | 원본 라이브러리 MIT. 기존 Saturn 합성 자료는 저장소 정책을 따른다. |

## 파일

합성 시나리오 48개와 기존 Saturn 패킷 48개는 [패킷 품질 재측정 원자료](../../handoff-packet-quality-v2/data/README.md)에 있다. 이번 실험의 Claude 원응답, Jev 호출별 사용량, 호출별 남김 확률과 판정은 로컬 비공개 원자료 한 파일에 수집했다. 원자료의 SHA-256은 `df481c86a2bf6416d81c1167d7b53d1cd60f97d32b0ad97ba26b68af44d06fbb`이다. 키 문자열은 원자료와 공개 파일에서 발견되지 않았다.

`scripts/02-process.py`가 원자료와 공개 시나리오를 다시 읽어 `results/summary.json`을 만든다. 공개 자료에는 질문별 원응답을 복사하지 않고 집계와 시나리오별 크기·상태만 남긴다. 원자료가 없는 환경에서는 다시 분석할 수 없으며, 같은 API 키와 로그인된 Claude CLI로 새 수집을 해야 한다. 외부 모델 출력은 재수집할 때 달라질 수 있다.

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| 로컬 원응답 JSONL | 조건별 Claude 응답, Jev 확률과 사용량 | `scripts/01-collect.mjs` |
| `results/summary.json` | 채점, 대응 차이, bootstrap 구간 | `scripts/02-process.py` |

| 자료 | 개수 | SHA-256 |
|---|---:|---|
| 기존 합성 시나리오 | 48 | `2a5cc1b6b2c8cc6c80fa2141f3a7d1afb4d5ac69feba38c1b27c7cc0c86e08f7` |
| 기존 Saturn 패킷 | 48 | `3356cc896cc61a6a6147b3f8ce2dcc77a8831f241d36aebe079a0cc873a101dc` |
| 이번 로컬 원응답 | 48쌍 | `df481c86a2bf6416d81c1167d7b53d1cd60f97d32b0ad97ba26b68af44d06fbb` |

원본 플러그인의 라이브러리를 `e3f262a7f4d42bd8dd32ced30d26176f7cb545b0`에서 빌드했다. 스크립트는 `FAST_JEV_SOURCE`가 그 checkout을 가리킬 때 빌드된 `dist/index.js`를 읽는다. `./run.sh collect`는 키를 표준입력에서 받고 터미널 에코를 끈다.

```sh
git clone https://github.com/tamaratran/fast-jev-compaction.git /tmp/saturn-compare-fast-jev
git -C /tmp/saturn-compare-fast-jev checkout e3f262a7f4d42bd8dd32ced30d26176f7cb545b0
cd /tmp/saturn-compare-fast-jev
npm ci --ignore-scripts
npm run build
```

## 필드

### 로컬 원응답 JSONL

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `scenario_id` | string | 없음 | 고유 | 합성 시나리오 번호 | `s01` |
| `ts_utc` | datetime | UTC | 필수 | 수집 시각 | `2026-10-05T11:59:47Z` |
| `compaction.stats` | object | 없음 | 성공 시 필수 | 호출 보존·삭제 수와 전달 크기 | `callsDropped` |
| `compaction.decisions` | array | 없음 | 성공 시 필수 | 호출별 확률과 조치 | `drop_call` |
| `conditions` | object | 없음 | 성공 시 필수 | 두 조건의 Sonnet 원응답과 맥락 크기 | `saturn-packet` |
| `error` | string | 없음 | 실패 시에만 | 수집 실패 원인 | `Jev request failed` |
