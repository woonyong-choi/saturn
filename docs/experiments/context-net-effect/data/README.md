# 데이터

| 경로 | 내용 |
|---|---|
| `raw/formal-<트랙>-<시드>-<조건>.json.gz` | trial 하나의 원자료. 입력 문장과 설정, 시각, 출력 원문(plain 출력 또는 TUI 화면), 숨긴 값, 채점, engine 기록 저장소에서 읽기 전용으로 뽑은 표(`inputs`, `sessions`, `runs_meta`, `usage`, `judgments`, `handoff_packets`, `handoff_packet_items`, `events`). 수집한 그대로이고 수정하지 않는다. |
| `raw/pilot-*.json.gz` | 실행기 확인용 예비 실행. 분석하지 않는다. |
| `raw/startup-failed/` | 첫 입력이 접수되기 전에 router 점검에서 멈춘 시도 34건. 분석에 쓰지 않는다. |
| `raw/invalid-engine-lost/` | engine 연결이 끊긴 Codex 시도 3건. 분석에 쓰지 않고 같은 시드와 조건으로 다시 수집했다. |
| `replay/<트랙>-<시드>-jev.jsonl` | 기록된 `compact` 요청을 다시 보낸 응답 원문. |
| `SHA256SUMS` | 위 파일의 SHA-256 |

router 키, provider 토큰, 설정 파일은 들어 있지 않다. 사용자 폴더에 Claude Code가 남긴 실행 기록은 포함하지 않는다. 원자료의 작업 폴더와 저장소는 worktree의 `.runtime/`에 두고 커밋하지 않는다(시드로 다시 만들 수 있다).

재현: `./run.sh process`, `./run.sh analyze`, `./run.sh verify`. 수집은 `./run.sh collect <claude|codex|claude-codex|codex-t16> formal`이며 로그인된 CLI와 키체인 `saturn-verify-router`가 필요하다.
