# 데이터

> [!WARNING]
> 회차별 원문 메시지와 실행 스크립트는 대부분 없다. 이 폴더는 남은 근거(지시서, 보고서, 로그 발췌)만 보관한다. 수치의 1차 근거는 [이슈 #194](https://github.com/woonyong-choi/saturn/issues/194)의 결과 댓글 8개다.

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | 실험 에이전트가 실험 worktree에서 Claude Code와 Codex CLI를 실행하고 관측을 이슈 댓글에 올렸다. 실험 5~8은 Codex 에이전트의 보고서와 실행 전체 로그가 남았다. 실험 1~4는 이슈 댓글만 남았다. |
| 수집 기간 | 2026-10-02~2026-10-02. 결과 댓글 8개의 작성 시각은 2026-10-01T15:36:26Z~2026-10-01T22:12:11Z다. |
| 개수 | 결과 댓글 8개. 지시서 4개, 보고서 4개, 실행 로그 4개 |
| 표본 여부 | 전수. 표본 추출 없이 실험마다 지시서에 정한 반복 횟수를 모두 실행했다. |
| 라벨 | 없음. 회차별 판정은 실험 에이전트가 붙였다. 사람이 다시 확인한 기록은 없다. |
| 알려진 문제 | 회차별 원자료가 유실됐다. 실험 3의 인용 일부가 틀렸고 실험 4는 항목 일부를 잘못 건너뛰었다. 실험 5가 모델 호출 상한을 넘겼다. 상세는 [보고서](../report.md) 한계 절이다. |
| 개인정보 | 지시서와 보고서에 사용자 홈 경로(`/Users/...`)가 있다. 계정 플랜 종류(`planType=pro`)가 있다. 토큰, 키, 인증 값은 없다(복사 전에 `token`, `key`, `secret`, `bearer`, `sk-` 패턴을 검색해 확인). 실행 로그에는 계정 이메일과 의존 패키지 저자 이메일이 있어 저장소에 넣지 않았다. |
| 라이선스 | 저장소 라이선스를 따른다. |

## 원자료 상태

| 대상 | 상태 | 비고 |
|---|---|---|
| 실험 1~4의 회차별 원문 메시지와 스크립트 | 유실 | 실험 worktree와 함께 지워졌다. 이슈 댓글 1~4만 남았다. |
| 실험 5~8의 드라이버, 회차별 관측 파일(`matrix-results.json`, `remaining-results.json`, `account-results.json`, `mcp-results.json`, `observations.jsonl`, `results.jsonl`, `tool-calls.jsonl` 등), 전용 `CODEX_HOME` | 유실 | 실험 worktree와 함께 지워졌다. 보고서가 인용한 파일 줄 번호는 확인할 수 없다. |
| 실험 5~8의 지시서 | 보관 | `raw/codex-194-{home,plug,mcp,prompt-path}-prompt.md` |
| 실험 5~8의 보고서 | 보관 | `raw/codex-194-{home,plug,mcp,prompt-path}-report.md`. 이슈 댓글 5~8과 같은 내용 |
| 실험 5~8의 Codex 실행 전체 로그 | 저장소 밖 보관 | 아래 표. 파일 크기 규칙(한 파일 50MB 이하)과 별개로 개인 정보가 있어 저장소에 넣지 않았다. |
| 실행 로그의 핵심 줄 | 보관 | `raw/key-lines.md` |
| 이슈 댓글 8개 | GitHub 보관 | 저장소에 사본을 두지 않았다. |

## 저장소 밖 실행 로그

비공개 경로다. 사용자 로컬 장치의 `~/workspace/woon/.local/orchestration/saturn-build/` 안에만 있고 저장소와 GitHub에 올라가 있지 않다. 이 경로의 파일은 다른 장치에서 읽을 수 없다.

| 실험 | 파일 | 크기 | SHA-256 |
|---|---|---|---|
| 5. 전용 `CODEX_HOME` | `~/workspace/woon/.local/orchestration/saturn-build/codex-194-home.log` | 3,070,349바이트 | `3cd4606bd74735c2fc13b8e26ecc1ef9009b1fb7d7036711c11a654ec1d37551` |
| 6. 구멍 막기 | `~/workspace/woon/.local/orchestration/saturn-build/codex-194-plug.log` | 2,963,403바이트 | `f4999c9ac27e862c94b2a76e1884d1c2d48f379a5fb585fe25811f7fe3067eec` |
| 7. MCP 준비 시점 | `~/workspace/woon/.local/orchestration/saturn-build/codex-194-mcp.log` | 3,393,037바이트 | `ddcfa97f84e1f53cbf68cddd9faadd4886b12dd9064b991eb8044592958a146b` |
| 8. MCP 묻기 경로 | `~/workspace/woon/.local/orchestration/saturn-build/codex-194-prompt-path.log` | 3,402,504바이트 | `a4fe82296bb2c8fe0e54abd4650b907a6c6aa6848ce18e53c0677f5963f71d18` |

로그는 Codex 에이전트(`gpt-5.6-luna`)의 실행 전체 기록이다. 에이전트가 읽은 파일, 실행한 명령, 만든 드라이버 코드, 드라이버 출력이 들어 있다. 로그에서 확인한 시험 thread의 모델 이름은 [env.json](../env.json)에 적었다. 같은 폴더의 `codex-194-rootcause-*`는 실험 9(원인 찾기, 진행 중)의 기록이라 이 폴더에 넣지 않았다.

## 파일

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| `raw/codex-194-home-prompt.md` | 실험 5 지시서 | 해당 없음(에이전트에게 준 지시서 사본) |
| `raw/codex-194-home-report.md` | 실험 5 보고서 | 해당 없음(에이전트 작성) |
| `raw/codex-194-plug-prompt.md` | 실험 6 지시서 | 해당 없음 |
| `raw/codex-194-plug-report.md` | 실험 6 보고서 | 해당 없음 |
| `raw/codex-194-mcp-prompt.md` | 실험 7 지시서 | 해당 없음 |
| `raw/codex-194-mcp-report.md` | 실험 7 보고서 | 해당 없음 |
| `raw/codex-194-prompt-path-prompt.md` | 실험 8 지시서 | 해당 없음 |
| `raw/codex-194-prompt-path-report.md` | 실험 8 보고서 | 해당 없음 |
| `raw/key-lines.md` | 실행 로그 4개의 핵심 줄 발췌(줄 번호 표기) | 해당 없음(손으로 발췌) |
| `SHA256SUMS` | `raw/` 파일의 SHA-256 | 해당 없음(`shasum -a 256`) |

복사한 지시서와 보고서는 원본과 바이트가 같다. `raw/key-lines.md`는 로그에서 줄을 옮겨 적은 것이고 긴 줄은 `...`로 줄였다. `scripts/`, `run.sh`, `results/`, `processed/`는 원본이 없어 만들지 않았다. 재현 절차는 [설계](../design.md)의 실험별 방법 표다.

## 필드

해당 없음. JSON Lines나 CSV 원자료가 없어 정의할 필드가 없다. 보고서 표의 회차별 관측 칸이 이슈 댓글과 에이전트 보고서에서 옮긴 값이다.
