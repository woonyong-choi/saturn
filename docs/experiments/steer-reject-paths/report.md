# 끼워 넣기 거절 경로: 실험 결과

## 요약

실제 Codex(codex-cli 0.158.0, `gpt-5.6-luna`)와 Saturn engine을 소켓으로 직접 불러, 끼워 넣기를 켠 실험 빌드에서 입력 B를 끼워 넣었다. 수집한 6개 조건 36회(`formal` 24회, `formal2` 12회)에서 불변식 세 가지(B가 `Applied`로 끝남, 표지 한 줄만 쓰임, 되돌아간 뒤 한 번만 전달됨)를 모두 지켰다(36/36, Wilson 95% [90.4, 100.0]). 검토 턴과 수동 압축 턴의 끼워 넣기는 확정 거절(`NotSent`)로 대기열 맨 앞에 돌아갔다가 그 턴이 끝난 뒤 새 턴으로 한 번 실행됐다(검토 3/3, 압축 18/18). 가설 H1~H6은 모두 채택이다.

한계가 둘 있다. 이 채택은 끼워 넣기를 켜도 된다는 판단이 아니다. 그리고 예비 실행 1건에서 압축 요청이 받아들여진 직후 60밀리초에 끼워 넣은 입력이 `Rejected`로 끝났다(유실). 같은 조건 15회(`steer_compact` 3, `steer_compact_burst` 12)에서는 되풀이되지 않았다. 예비 실행은 표본이 아니므로 가설 결과에 넣지 않았고, 결함 [#PENDING_DEFECT](https://github.com/woonyong-choi/saturn/issues)로 등록했다. 턴 끝 경합(`steer_race`)은 끼워 넣기를 시도한 시점의 활성 턴 판단이 어긋난 경로를 실제로 만들지 못했다(`fallback_new_turn` 0회). 그 경로는 가짜 provider 시험으로만 확인했다. 결과를 모르는 전달의 재전송 금지와 연결 교체는 실제로 만들지 못했다(미측정).

## 방법

| 항목 | 값 |
|---|---|
| 설계 | [실험 설계](design.md)(수집 전 커밋 `62fbda7`, 추가 수집 설계 커밋은 설계 문서의 "추가 수집(2차)") |
| 환경 | [env.json](env.json) |
| 빌드 | 기준 커밋의 `saturn-engine` 릴리스 빌드에서 `providers/codex.rs`의 `STEER_VERIFIED`만 참으로 바꾼 실험 빌드([steer-verified.patch](steer-verified.patch)). 제품 코드는 바꾸지 않았다. |
| 표본 | `steer_ok` 3, `steer_review` 3, `steer_compact` 3, `steer_compact_late` 3, `steer_race` 12(수집 `formal` 24회), `steer_compact_burst` 12(`formal2`). 예비 실행 `pilot2` 4건은 분석하지 않는다. |
| 호출 | provider 턴 102회(설계 상한 80 + 추가 수집 40 = 120), router 호출 0회. 키 문자열이 든 파일 0개(`key_files_with_key`). |
| 재현 | `scripts/analyze.py`가 [results/summary.json](results/summary.json)을 만든다. 두 번 돌려 바이트가 같았다(`--check`). |

## 설계와 다른 점

| 변경 | 시점 | 이유 | 결론에 미친 영향 |
|---|---|---|---|
| 수동 압축 조건을 `steer_compact`(압축 요청이 받아들여진 직후)와 `steer_compact_late`(1.0초 뒤) 둘로 나누고 반복을 더했다. | 수집 전, 예비 실행 뒤 | 예비 실행 1건이 압축 시작 직후 `Rejected`로 끝났다. | 설계 문서에 수집 전에 적었다. |
| `steer_compact_burst` 12회를 추가 수집했다. | 첫 수집 뒤, 추가 수집 설계 커밋 뒤 | 같은 조건이 확률적으로 갈리는지 보려 했다. | `formal`의 결과와 기준을 바꾸지 않고 따로 판정했다. |
| 예비 실행에서 실행기가 압축 요청의 `Applied`를 기다리지 않고 작업이 실행 중으로 보이는 시점을 기다렸다. | 예비 실행 | 압축 시작 시각을 맞추려는 첫 구현이었다. | 이 예비 실행만 입력이 압축 요청과 같은 시각(`Applied`와 동시)에 갔다. 정식 수집은 `Applied`를 기다린 뒤 바로 보냈다. |

## 결과

### 가설

| 가설 | 조건 | 결과 | Wilson 95% | 판정 |
|---|---|---|---|---|
| H1 일반 실행에 끼워 넣으면 끼워 넣어진다 | `steer_ok` | 3/3 모두 `steered`(`steered_run` 있음) | [43.8, 100.0] | 채택 |
| H2 검토 턴 거절 뒤 대기, 한 번 실행 | `steer_review` | 3/3 | [43.8, 100.0] | 채택 |
| H3 압축 직후 | `steer_compact` | 3/3 | [43.8, 100.0] | 채택 |
| H4 압축이 시작된 것을 안 뒤 | `steer_compact_late` | 3/3 | [43.8, 100.0] | 채택 |
| H5 턴 끝 앞뒤 12점 | `steer_race` | 12/12 | [75.7, 100.0] | 채택 |
| H6 압축 시작 경합 12회 | `steer_compact_burst` | 12/12 | [75.7, 100.0] | 채택 |

표본이 작아 구간이 넓다. 채택은 "속성 위반이 없었다"는 뜻이다.

### B가 끝난 경로

| 조건 | 경로 | 거절 문구(provider 응답, id는 `<id>`로 가림) |
|---|---|---|
| `steer_ok`(3) | `steered` 3 | 없음 |
| `steer_review`(3) | `refused_queued` 3 | `expected active turn id <id> but found <id>` 3 |
| `steer_compact`(3) | `refused_queued` 3 | `cannot steer a compact turn` 3 |
| `steer_compact_late`(3) | `refused_queued` 3 | `cannot steer a compact turn` 3 |
| `steer_compact_burst`(12) | `refused_queued` 12 | `cannot steer a compact turn` 12 |
| `steer_race`(12) | `steered` 5, `queued_normal` 7 | 없음 |

- 검토 턴의 거절 문구는 `cannot steer a review turn`이 아니라 틀린 `expectedTurnId` 문구였다. Saturn이 `turn/started`의 턴 id를 활성 id로 쓰는데 검토 턴에서는 이 id가 서버의 활성 id와 달라서다([Codex provider 실측](../codex-provider-behavior/report.md)이 이미 적은 차이). 어느 문구든 어댑터는 `NotSent`로 읽고 입력은 같은 ID로 대기열 맨 앞에 돌아갔다. 사유가 우연이라 검토 턴의 거절은 문구가 아니라 `expectedTurnId` 불일치에 기대고 있다.
- `steer_race`의 7회는 B가 끝난 턴 뒤에 도착해 처음부터 새 턴으로 갔다(`Judging → Delivering → Applied`). 활성 턴이 있다고 믿는 채 끼워 넣다가 `no active turn`을 받은 회차는 없었다.
- 모든 반복에서 `notes.txt`의 표지는 정확히 한 줄이다(유실·중복 0).

### 가짜 provider 회귀(실제 확인이 아니다)

[steer_rejected.rs](../../../saturn-terminal/engine/src/lifecycle/steer_rejected.rs)에 시험 3개를 더했고 통과했다. 끼워 넣기의 결과를 모르면(`Unknown`) 다시 끼워 넣거나 새 턴으로 보내거나 대기로 되돌리지 않고 `전달 중`으로 남는다. 턴이 끝난 뒤 도착한 확정 거절은 같은 입력이 새 턴으로 한 번 간다. 턴이 끝난 뒤 도착한 수락 응답은 다시 보내지 않는다. 기존 시험(`refused_steer_*`, `steer_without_active_turn_*`)은 확정 거절과 활성 턴 없음 경로를 이미 덮는다.

## 완료 조건 대조

| 조건 | 상태 |
|---|---|
| 거절 후 같은 입력 ID가 유실/중복 없이 다음 대기로 이동한다. | 정식 수집 36/36 통과. 예비 실행 1건에서 유실(`Rejected`)을 보았고 결함 이슈로 분리했다. 그 결함이 닫히기 전에는 채워졌다고 보지 않는다. |
| 실제 Codex 특수 상태의 지원/미지원/거절을 구분하고 Claude 결과와 섞지 않는다. | Codex: 검토 턴과 수동 압축 턴의 거절 확인. Claude: 이 실험에서 측정하지 않았다(`STEER_VERIFIED` 거짓, 특수 턴 없음). 미측정. |
| 늦은 ack·연결 교체·턴 종료 경합을 가짜 provider 회귀와 실제 관측으로 확인한다. | 가짜 provider 시험 3개 통과. 실제 관측은 턴 종료 경합 12점에서 위반 없음(경합 경로를 만들지 못함). 늦은 ack와 연결 교체의 실제 관측은 만들지 못해 미측정. |

## 한계와 다음 실험

- 결과를 모르는 전달은 실제 provider로 만들지 못했다. 필요한 선행 작업: provider가 `turn/steer` 응답을 10초 넘게 주지 않는 상태를 만드는 방법(가짜 app-server 외). 입력 계약은 `ProviderError::Unknown`.
- 턴 끝 경합의 `NoActiveTurn` 경로는 실제로 만들지 못했다. 시점을 0.5초 간격으로 흩뿌렸지만 Saturn은 턴 끝 이벤트를 받은 뒤라서 활성 턴이 없다고 알았다. 만들려면 Codex의 `turn/completed` 이벤트 지연을 흉내 내는 중계가 필요하다.
- Claude의 끼워 넣기 거절·특수 상태는 이 실험 밖이다.
- 끼워 넣기를 켜는 판단은 [#27](https://github.com/woonyong-choi/saturn/issues/27)과 결함 해결 뒤에 한다.
