# 끼워 넣기 거절 경로: 실험 설계

| 항목 | 값 |
|---|---|
| 이슈 | [#5](https://github.com/woonyong-choi/saturn/issues/5) |
| 관련 설계 | [입력 처리](../../design/input-handling.md), [provider 연결과 session](../../design/providers-and-sessions.md) |
| 사전 데이터 | 실제 Codex로 이 실행기를 한 번씩 돌려 본 예비 실행(`steer_ok`, `steer_review`, `steer_compact`, `steer_race` 오프셋 1.0초 각 1회와, 같은 `steer_review`를 앞서 3회)을 수집 전에 열람했다. 예비 실행은 실행기 수정에만 쓰고 표본에 넣지 않으며 분석하지 않는다. 예비 실행에서 `steer_compact`의 입력이 `Rejected`로 끝나는 것을 봤다. 그래서 가설 H3을 그 현상이 사라질 때만 채택되게 썼다. 호출 수는 아래 상한에 센다. |

## 질문

Codex가 끼워 넣기를 확정 거절하는 상태(검토 턴, 수동 압축 턴, 턴이 막 끝난 경합)에서 Saturn이 같은 입력 ID를 유실이나 중복 없이 다음 대기로 옮기는가. 끼워 넣기가 일반 실행에서 받아들여지는 기존 관측은 대조로 한 번 더 둔다. 결과를 모르는 전달은 다시 보내지 않는다는 규칙은 실제 provider로 만들 수 없으므로 가짜 provider 회귀로만 확인하고 그렇게 구분해 보고한다.

현재 제품은 `STEER_VERIFIED`가 거짓이라 끼워 넣기를 대기로 바꾼다. 이 실험은 그 경로가 아니라 끼워 넣기를 켠 경로를 잰다. 그래서 수집용 빌드는 `providers/codex.rs`의 `STEER_VERIFIED` 한 줄만 참으로 바꾼 실험 빌드이고([steer-verified.patch](steer-verified.patch)), 이 변경은 저장소에 넣지 않는다. 같은 입력에 router 판단을 섞지 않으려고 입력은 `skip_relation`으로 접수한 뒤 `SendNow`로 끼워 넣기를 시도한다. router 호출은 없다.

## 가설

가설 하나는 조건의 모든 반복이 다음 불변식을 모두 지킬 때만 채택한다. 하나라도 어기면 그 조건은 기각이다. 표본이 작아 구간은 넓고, 판정은 속성 위반이 있었는지로 한다.

불변식(입력 B는 `notes.txt`에 `MARK-B-<반복>` 한 줄을 덧붙이는 입력이다):

1. B의 마지막 상태는 `Applied`이고 `Rejected`나 `Held`로 끝나지 않는다.
2. `notes.txt`에 `MARK-B-<반복>`이 정확히 한 줄이다(유실과 중복이 없다).
3. B가 `Delivering` 뒤 `Queued`로 돌아왔다면 그 뒤 `Delivering`은 한 번이고, 모든 상태 전이는 같은 입력 ID에서 일어난다.

| 가설 | 예측 | 조건(반복) |
|---|---|---|
| H1 | 일반 실행(60줄 글쓰기 중 4초 뒤)에 끼워 넣은 B는 끼워 넣어져 `Applied`가 되고 `inputs.steered_run`이 채워지며 불변식 1~2를 지킨다. 대조다. | `steer_ok`(3) |
| H2 | 검토 턴(`/review`가 돈 지 4초 뒤)에 끼워 넣은 B는 거절되어 `Queued`로 돌아가고, 검토가 끝난 뒤 새 턴으로 한 번 실행되어 불변식을 모두 지킨다. | `steer_review`(3) |
| H3 | 수동 압축 턴(`/compact`가 `Applied`가 된 직후)에 끼워 넣은 B는 불변식을 모두 지킨다. | `steer_compact`(3) |
| H4 | 수동 압축 턴이 시작된 것을 알고 있을 때(`Applied` 1.0초 뒤)에 끼워 넣은 B는 불변식을 모두 지킨다. | `steer_compact_late`(3) |
| H5 | 짧은 턴(`ok` 한 단어)이 끝나는 앞뒤로 끼워 넣는 시점을 1.0초부터 0.5초 간격 12점으로 흩뿌려도 B는 모든 점에서 불변식을 지킨다. | `steer_race`(12) |

H1~H5의 결과는 B가 어느 경로로 끝났는지도 센다. 분류는 `inputs.steered_run`이 있으면 `steered`, 끼워 넣기 거절 로그가 있으면 `refused_queued`, `Delivering`(처분 `Steer`) 뒤 바로 `Applied`이고 `steered_run`이 없으면 `fallback_new_turn`, 처음부터 처분이 `Queue`로 끝났으면 `queued_normal`이다.

가짜 provider 회귀(실제 확인이 아니다). 새 시험 3개로 확인한다.

| 확인 | 시험 |
|---|---|
| 끼워 넣기의 결과를 모르면(`Unknown`) 다시 끼워 넣지도, 새 턴으로 보내지도, 대기로 되돌리지도 않는다. | `steer_with_an_unknown_result_is_not_sent_again_or_requeued` |
| 턴이 끝난 뒤에 확정 거절(`NotSent`)이 도착해도 같은 입력이 새 턴으로 한 번 간다. | `refused_steer_answered_after_the_turn_ended_still_takes_one_turn` |
| 턴이 끝난 뒤에 늦은 수락 응답이 도착해도 다시 보내지 않는다. | `accepted_steer_answered_after_the_turn_ended_is_not_sent_again` |

## 설계

| 항목 | 값 |
|---|---|
| 호출 | Codex `gpt-5.6-luna`만. Claude는 `STEER_VERIFIED`가 거짓이고 검토·압축 같은 특수 턴이 없어 이 실험에서 측정하지 않으며 `미측정`으로 보고한다. 실제 결과를 모르는 전달과 연결 교체는 실제 provider로 만들지 못해 `미측정`이다. |
| 호출 상한 | provider 턴 80회 이하(예상 60회: `steer_ok` 6, `steer_review` 6, `steer_compact` 9, `steer_compact_late` 9, `steer_race` 36). router 호출 0회. 닿으면 멈추고 보고한다. |
| 실행기 | `scripts/collect.py`가 반복마다 새 `SATURN_HOME`과 새 git 연습 폴더(미커밋 변경 하나, 검토 대상)에서 `saturn-engine`을 띄우고 소켓으로 `Attach`, `SubmitInput`, `SendNow`를 보낸다. 모델은 `-c model.default=codex/gpt-5.6-luna`, 권한 모드는 `full`이다. 키는 키체인에서 수집 명령 안에서만 읽어 engine 환경 변수로 넘기고 출력하지 않는다. engine은 이 실행기가 띄운 PID만 종료한다. |
| 기록 | 소켓 알림 전체, 기록 저장소의 `inputs`와 `runs`(읽기 전용), `notes.txt`, engine 로그 중 끼워 넣기 줄을 `data/raw/<실행>/<조건>-<번호>.json.gz`에 둔다. 끝에 연습 폴더와 홈 전체에서 키 문자열을 찾아 파일 수를 센다(`key_files_with_key`). |
| 눈가림 | 해당 없음(조건이 결정적이다). 입력 문장은 조건마다 고정이다. |
| 중단 기준 | 같은 조건에서 engine이 시작하지 못하거나 provider가 턴을 `failed`로 끝내 B를 보내기 전에 실험이 틀어지면 그 반복은 `invalid`로 따로 보존하고 같은 조건·번호로 다시 수집한다(예비 실행처럼 B를 보낸 뒤의 실패는 되풀이하지 않는다). |
| 재현 | `scripts/analyze.py`가 원자료만으로 표를 만든다. 두 번 돌려 바이트 일치를 확인한다. |

수집 뒤에 기준을 바꾸지 않는다. 바꿔야 하면 그 수집은 탐색으로만 보고하고 다음 실험으로 다시 등록한다.

## 판정

H1~H5는 불변식을 모두 지키면 채택, 하나라도 어기면 기각이다. 기각된 조건은 결함 이슈로 등록한다. 이 실험의 채택은 끼워 넣기를 켜도 된다는 뜻이 아니고, 켜는 판단은 [#27](https://github.com/woonyong-choi/saturn/issues/27)과 이 실험의 모든 조건이 통과한 뒤에 한다.

## 추가 수집(2차): 압축 시작 경합

첫 수집(`formal`)이 끝난 뒤에 등록한다. 예비 실행에서 `steer_compact` 1건이 `Rejected`로 끝났고(압축 요청이 받아들여진 직후 60밀리초에 끼워 넣은 입력이 활성 턴 없음으로 보인 뒤 새 턴이 `ActiveTurnNotSteerable`로 거절됨), `formal`의 같은 조건 3건은 모두 통과했다. 같은 조건이 확률적으로 갈리는 것으로 보여 반복을 늘려 다시 재는 조건을 더한다. `formal`의 결과와 기준은 바꾸지 않고 따로 보고한다.

| 가설 | 예측 | 조건(반복) |
|---|---|---|
| H6 | 수동 압축 요청이 받아들여진 직후(`Applied`와 같은 시각)에 끼워 넣은 B는 불변식 1~3을 모두 지킨다. `steer_compact`와 같은 흐름을 12번 되풀이한다. | `steer_compact_burst`(12) |

호출 상한을 provider 턴 40회 더한다(12반복 × 3턴 = 36). 수집은 `data/raw/formal2/`에 둔다. 불변식을 어긴 반복이 하나라도 있으면 H6은 기각이고 결함 이슈를 등록한다.
