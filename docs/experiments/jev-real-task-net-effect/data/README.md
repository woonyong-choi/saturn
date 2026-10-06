# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | `run.sh inventory`와 `run.sh collect`의 최초 사전 검사, 그 뒤 봉인한 `02-source-replay.py`의 공개 과제 #577 source 재생 |
| 수집 기간 | 2026-10-07~2026-10-07 |
| 개수 | 확인 평가 source 0개·대응쌍 0개. 별도 기능 진단 source 2개, 무효 J target 1건·유효 대응쌍 0쌍, source provider 입력 2회·target 입력 2회 |
| 표본 여부 | 확인 평가 80쌍은 미등록. 기능 진단 공개 이슈 24건을 전수 감사하고 #577 하나를 재생 |
| 라벨 | 없음 |
| 알려진 문제 | R/J target 실행기와 조건이 아직 사전등록되지 않아 대응쌍 실행 불가 |
| 개인정보 | source 원응답은 Git 무시 대상 `.runtime/jr/s1`에만 보존하고 키 일치 문자열 0건을 확인했다 |
| 라이선스 | 공개 Saturn 이슈와 저장소 코드에 기초한 재생. provider 원응답은 공개하지 않는다 |

## 파일

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| `results/summary.json` | 가용성 검사 결과와 실측 미실행 상태 | `run.sh inventory`, 수동으로 출력 대조 |
| `data/eligibility-audit.csv` | 2026-10-05~2026-10-06에 생성·종결된 공개 이슈 24건의 첫 진단 선정과 미선정 이유 | 공개 이슈 목록을 수집 전 전수 대조 |
| `data/SHA256SUMS` | 공개 원자료가 없어 빈 해시 목록. s1 해시는 `summary.json`에 기록 | 원자료 공개 시 생성 |
| `.runtime/jev-real-task-effect/manifest.json` | 향후 source와 과제의 비공개 등록 파일 | 수집 전 작성 |
| `.runtime/jr/s1/source.raw.json` | #577의 새 재생 source 알림 원본. SHA-256은 `summary.json` 참조 | `02-source-replay.py` |
| `.runtime/jr/s1/home/saturn.db` | source 사건·사용량과 선행 대화 원본. SHA-256은 `summary.json` 참조 | Saturn engine |
| `.runtime/jr/source-work-diff.json` | 첫 target 실패 뒤 s1 원본과 pristine t1J 작업 폴더의 변경 경로 목록. 파일 본문 없음 | 읽기 전용 경로·해시 대조 |
| `.runtime/jr/s2/{home,work,seal.json}` | 공통 실제 작업 경로에서 재생해 봉인한 두 번째 기능 진단 source. 해시는 `summary.json` 참조 | `04-physical-workdir.py source` |
| `.runtime/jr/t2{J,R}/{home,work,target.raw.json}` | 다음 사전등록 뒤 별도 실행할 기능 진단 target. 현재 없음 | `04-physical-workdir.py target-j`, `target-r` |
| `.runtime/jev-real-task-effect/raw/{과제}-{조건}.json` | 향후 확인 평가의 engine 알림과 기록 행 원본 | 수집기 미구현 |

봉인 전 예비 source 시도 1건은 `.runtime/jr`의 무시 대상에 남겼다. `RouterKey` 거절로 provider 작업과 파일 변경은 없었다. 이 자료는 기능 진단이나 확인 평가의 원자료가 아니다.

봉인 뒤 `s1`은 Codex `gpt-5.6-luna`의 읽기 전용 선행 대화다. `ToolResult` 13건, 총 본문 1,219,097바이트를 남겼고 작업 폴더는 바뀌지 않았다. source 원자료는 기능 진단의 출발점일 뿐 80쌍 확인 평가 표본이 아니다.

첫 J target은 DB가 s1 원본 작업 경로를 유지해 원본을 수정한 무효 진단이다. `.runtime/jr/t1J`와 `.runtime/jr/t1R`은 재사용하지 않는다. J 원자료와 `.runtime/jr/source-work-diff.json`의 변경 경로 목록은 무시 대상에 보존한다. R 입력은 보내지 않았다. s2 source는 별도 사전등록 뒤 봉인했고 작업 폴더가 시작 상태와 같았다. `t2J`·`t2R`은 아직 호출하지 않았다.

## 필드

### `results/summary.json`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `real_source_snapshots` | integer | 개 | 0 이상 | 독립 확인 평가에 등록된 실제 과제 source 수 | `0` |
| `diagnostic_source_replays` | integer | 개 | 0 이상 | 별도 공개 과제 새 재생 source 수 | `1` |
| `runnable_pairs` | integer | 쌍 | 0 이상 | 봉인된 source에서 실행 가능한 대응쌍 수 | `0` |
| `provider_calls` | integer | 회 | 0 이상 | 최초 사전 검사·s0·s1·첫 무효 J target·s2를 합한 provider 입력 수 | `4` |
| `effect_status` | string | 없음 | `not_measured` | 효과 가설의 측정 상태 | `not_measured` |

기능 진단과 80쌍 확인 평가의 원자료가 생기면 출처, 제외 사유, 개인정보 처리, 전체 필드 계약을 별도 봉인 자료와 보고서에 기록한다. 지금 파일은 효과 실험의 원자료가 아니다.
