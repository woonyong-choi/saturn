# 데이터

| 경로 | 내용 |
|---|---|
| `raw/formal/<조건>-<번호>.json.gz` | 첫 수집 24건. 소켓 알림 전체(`messages`), 입력별 상태 전이(`inputs_seen`), 기록 저장소의 `inputs`와 `runs`(읽기 전용으로 뽑음), `notes.txt`, engine 로그 중 끼워 넣기 줄, 접수한 입력 수(`provider_calls`), 키 문자열이 든 파일 수(`key_files_with_key`). 수집한 그대로이고 수정하지 않는다. |
| `raw/formal2/<조건>-<번호>.json.gz` | 압축 시작 경합 추가 수집 12건. 형식은 같다. |
| `raw/pilot2/` | 실행기 확인용 예비 실행 4건. 분석하지 않는다. `steer_compact-1`이 입력 `Rejected`로 끝난 1건이고 engine 로그의 `provider rejected before send: ... ActiveTurnNotSteerable { turn_kind: Compact }`가 거절 사유다. |
| `SHA256SUMS` | 위 파일의 SHA-256 |

router 키, provider 토큰, 설정 파일은 들어 있지 않다. 작업 폴더와 기록 저장소는 worktree의 `.runtime/s5/`에 두고 커밋하지 않았다.

재현: `python3 scripts/analyze.py`, `python3 scripts/analyze.py --check`. 수집은 `scripts/collect.py <formal|formal2> [조건...]`이고 [steer-verified.patch](../steer-verified.patch)를 적용해 릴리스 빌드한 `saturn-engine`, 로그인된 `codex`, 키체인 `saturn-verify-router`가 필요하다.
