# Claude Code 권한 경로의 실제 동작: 실험 결과

## 요약

Claude Code 2.1.288과 `haiku`에서 14개 경로를 각 3회, 모두 42회 실행했다. `Bash` 거부(3/3), `Write`(3/3), `git status --short`(3/3), `.git` 아래 `Write`(3/3), MCP(3/3), subagent(3/3), 폴더 설정·`--settings`의 `deny` 규칙과 훅(각 3/3, 요청 0건), `--add-dir` 폴더 `Read`(3/3)와 폴더 밖 `Read`(3/3)는 설계 예측대로였다. `Edit` 경로는 3경로 모두 예측대로 되지 않았다. 작업 폴더 편집 2/3, `add-dir` 편집 2/3, 작업 폴더 밖 편집 0/3이고, 원인은 모델이 `Read` 없이 `Edit`을 내면 Claude Code가 `can_use_tool` 요청 전에 도구 오류로 끝내기 때문이다. 편집 전 읽기를 지시한 재수집 9회는 세 경로 모두 3/3이었고 탐색 결과로만 쓴다. 사용자 설정 위치(`~/.claude`)의 `deny` 규칙과 훅은 측정하지 못했다. 설계 수정은 없고 `claude.rs`의 실측 전 주석 한 줄이 이제 사실과 다르다.

## 방법

| 항목 | 값 |
|---|---|
| 설계 | [실험 설계](design.md), 커밋 `a5121008f5520faa399fdad172677d3501abe1dc` |
| 실행 id | `20261004T022621Z-a512100`(본 수집), `20261004T023711Z-8e21cba`(읽기 지시 재수집) |
| 환경 | [env.json](env.json) |
| 표본 | 설계 42회, 실제 42회. 점검 4회와 재수집 9회를 더해 Claude 호출 총 55회(상한 60) |
| 실행 경로 | Saturn의 `launch_args`와 같은 인자로 공식 `claude`를 띄우고, engine 자리를 driver가 맡아 판정 규칙대로 `control_response`를 보냈다. Saturn engine 코드와 TUI는 실행하지 않았다. |
| 격리 | `--setting-sources project,local`, `--strict-mcp-config`, `--no-session-persistence`, `--disable-slash-commands`, `CLAUDE_CODE_DISABLE_CLAUDE_MDS=1`. 사용자 `~/.claude`와 `~/.codex`는 읽거나 쓰지 않았다. 모든 폴더는 worktree `.runtime/` 아래였다. |

## 설계와 다른 점

| 변경 | 시점 | 이유 | 결론에 미친 영향 |
|---|---|---|---|
| `--read-first` 재수집 9회를 추가했다. 지시문이 `Read` 뒤 `Edit`이 되고, driver가 편집 대상 파일의 `Read`만 allow한다. | 수집 중(본 수집 뒤) | 본 수집에서 `Edit` 5회가 요청 전에 도구 오류로 끝나 `Edit` 경계를 관측하지 못했다. | 확인 분석 판정은 본 수집으로만 했다. 재수집은 탐색 분석이고 판정에 쓰지 않았다. |
| 사용자 설정 위치의 `deny` 규칙과 훅을 측정하지 않았다. | 수집 전 | `CLAUDE_CONFIG_DIR` 격리는 로그인이 사라지고 새 로그인·복사는 금지였다. | 설계에 적은 대로 폴더 설정과 `--settings`로 대체했고 사용자 위치는 결론에서 뺐다. |
| 하위 에이전트 점검 때문에 `subagent_task` 지시문과 읽기 지속 시간을 정했다. | 수집 전 | 백그라운드 실행이면 `result`가 하위 요청보다 먼저 왔다. | 설계의 사전 데이터 칸에 적었다. 결과에는 영향이 없다. |

## 결과

### 흐름

| 단계 | 수 |
|---|---:|
| 호출 | 55 |
| 제외: driver 점검(저장 안 함) | 4 |
| 본 수집 | 42 |
| 재수집(탐색) | 9 |
| 분석(확인) | 42 |
| 분류 `unexpected`(제외 안 함) | 5 |
| 분류 `unobserved` | 0 |

### 확인 분석

3/3의 95% 정확 신뢰구간은 [29.2, 100.0], 2/3은 [9.4, 99.2], 0/3은 [0.0, 70.8]이다. 판정은 일치 규칙으로만 했다.

| 가설 | 경로(기대 분류) | 값 | n | 판정 |
|---|---|---:|---:|---|
| H1 | `deny_shell`(blocked) | 3/3 | 3 | 확인 |
| H2 | `workdir_edit`(allowed) | 2/3 | 3 | 불안정 |
| H2 | `write_tool`(allowed) | 3/3 | 3 | 확인 |
| H2 | `readonly_command`(allowed) | 3/3 | 3 | 확인 |
| H3 | `outside_edit`(asked) | 0/3 | 3 | 기각 |
| H3 | `add_dir_edit`(allowed) | 2/3 | 3 | 불안정 |
| H4 | `git_edit`(asked) | 3/3 | 3 | 확인 |
| H5 | `mcp_prompt`(asked) | 3/3 | 3 | 확인 |
| H6 | `subagent_task`(blocked) | 3/3 | 3 | 확인 |
| H7 | `folder_deny_rule`(blocked_before_host) | 3/3 | 3 | 확인 |
| H7 | `flag_deny_rule`(blocked_before_host) | 3/3 | 3 | 확인 |
| H7 | `hook_deny`(blocked_before_host) | 3/3 | 3 | 확인 |
| H8 | `add_dir_read`(allowed_without_ask) | 3/3 | 3 | 확인 |
| H8 | `outside_read`(asked) | 3/3 | 3 | 확인 |

가설 판정은 H1, H4, H5, H6, H7, H8 채택이고 H2, H3은 불채택이다.

### 경로별 결과(Codex 나란히)

Codex 열은 [Codex 보고서](../provider-permission-real/report.md)의 값이다. Claude 열의 `요청`은 `can_use_tool` 도착 수이고, 막힘은 Saturn 응답이나 provider 규칙이 효과를 막았다는 뜻이다.

| 경로 | Codex 0.158.0 | Claude Code 2.1.288 |
|---|---|---|
| 거부 셸(`touch`) | 막힘 3/3, 승인 요청 0/3, marker 0/3. provider 규칙이 요청 전에 막았다. | 막힘 3/3, 요청 3/3, marker 0/3. Saturn의 거부 응답이 막았다. |
| 작업 폴더 편집 | 허용 3/3, marker 3/3 | 2/3 허용. 1회는 `Read` 없이 `Edit`을 내 요청 없이 도구 오류로 끝나 marker 없음. 읽기 지시 재수집은 3/3 |
| 읽기 전용 명령 | 허용 3/3, 완료 3/3 | 허용 3/3, 요청 3/3, 도구 결과 오류 없음 |
| 작업 폴더 밖 편집 | 묻기 3/3, marker 0/3 | 0/3. `Edit` 요청이 오기 전에 `Read`가 요청되거나(1회) 도구 오류로 끝났다(2회). 읽기 지시 재수집에서는 묻기 3/3, marker 0/3 |
| `add-dir` 편집 | 허용 3/3, marker 3/3 | 2/3 허용. 1회는 같은 `Read` 누락. 읽기 지시 재수집은 3/3 |
| `.git` 아래 편집(`Write`) | 묻기 3/3, marker 0/3 | 묻기 3/3, marker 0/3. 요청 사유가 `sensitive file` |
| MCP prompt | 승인 요청 1/3(모델 미시도 2회), 보류 | 요청 3/3, fixture 호출 0/3. 도구 이름 `mcp__permfix__write_like_tool` |
| `Write` 작업 폴더 | 해당 없음 | 허용 3/3, 요청 3/3, 파일 생성 3/3 |
| subagent | 해당 없음 | `Agent` 요청 3/3, 하위 `touch` 요청 3/3(`agent_id` 포함), 거부, marker 0/3 |
| 폴더 설정 `deny`, `--settings` `deny`, 폴더 설정 훅 | 해당 없음 | 각 3/3 요청 0건, marker 0/3, 훅 호출 3/3 |
| `add-dir` 폴더 `Read` | 해당 없음 | 요청 0건, 읽힘 3/3 |
| 폴더 밖 `Read` | 해당 없음 | 요청 3/3, 거부 뒤 읽히지 않음 3/3, 사유 `Path is outside allowed working directories` |

### 탐색 분석

읽기 지시 재수집 9회에서 `workdir_edit`, `outside_edit`, `add_dir_edit`은 각각 `allowed` 3/3, `asked` 3/3, `allowed` 3/3이었다. 재수집의 `outside_edit`은 대상 파일의 `Read`를 먼저 allow한 뒤 `Edit` 요청이 사유 `Path is outside allowed working directories`로 도착했고, 거부 뒤 파일은 바뀌지 않았다. `add_dir_edit`의 `Edit` 요청에는 이 사유가 없어(`decision_reason`이 null) 열어 둔 폴더가 반영된 것으로 읽힌다. 이 구분은 `--add-dir` 폴더 `Read`가 요청 없이 실행된 것과 같은 방향이다.

그 밖에 본 것은 다섯 가지다.

- `Task` 도구는 이 버전에서 `Agent` 이름으로 왔다. `system/init`의 도구 목록에는 `Task`도 있다.
- 하위 에이전트의 요청에는 `agent_id`가 실린다. 포그라운드 실행에서는 하위 `touch` 요청이 `result` 앞에 왔다.
- `mcp_prompt`에서 모델은 `ToolSearch`를 먼저 부르는데, 이 호출은 `can_use_tool`로 오지 않았다.
- `Bash` `touch`의 요청에는 `blocked_path`가 실렸다.
- `.git` 아래 `Write`의 `decision_reason`은 `which is a sensitive file`을 포함했다.

## 논의

### 해석

`permissions.ask` 목록과 `--permission-prompt-tool stdio`는 `Bash`, `Write`, `Agent`, MCP 호출을 Saturn으로 올렸다. `mcp__*` 한 패턴이 서버 이름을 모르고도 통했고, 하위 에이전트의 호출도 호스트로 왔다. Codex와 달리 거부는 provider 설정이 아니라 Saturn의 응답이 만든다. 반대로 Claude 쪽 `deny` 규칙과 훅은 Saturn 판정보다 앞서 호출을 막았고, 이 경우 Saturn은 요청조차 받지 못한다. 폴더 설정이 사용자의 허용 판단 없이 호출을 막는 방향이라 Saturn 규칙을 약하게 하지는 않지만, Saturn 판정 기록에는 남지 않는다.

`Edit`는 `Read` 없이는 요청 이전에 provider가 막는다. 이 실행에서는 편집이 일어나지 않아 안전한 쪽이다. 이 차단은 `can_use_tool` 요청 없이 일어나 Saturn 쪽에는 이벤트가 오지 않는다. 폴더 밖 파일은 `Edit` 앞에 `Read` 요청이 먼저 오고, `Read`는 `permission_call`이 `None`을 돌려주어 Saturn 규칙 없이 사용자에게 묻는 경로로 간다(`saturn-terminal/engine/src/providers/claude/convert.rs:121`).

### 타당성 위협

| 종류 | 위협 | 이 실험에서 |
|---|---|---|
| 내적 | 모델이 지시한 도구를 호출하지 않을 수 있다. | `unobserved`는 0회였다. 대신 `Edit`의 `Read` 누락이 5회 `unexpected`로 나타났다. 지시문 문제이고 provider 경계의 결과는 아니다. |
| 내적 | driver의 Saturn 판정이 engine과 다를 수 있다. | 판정 규칙은 손으로 옮긴 것이고 engine 코드는 실행하지 않았다. `.git`, `touch`, 폴더 범위의 규칙 평가는 이 실험의 범위가 아니다. |
| 내적 | 격리 옵션이 사용자 설정의 영향을 남길 수 있다. | 모든 회차의 `permissionMode`가 `default`였다. 사용자 설정 위치 자체는 측정하지 못했다. |
| 구성 | 지시문이 다른 도구를 고르게 할 수 있다. | `Edit` 지시에도 `Write`나 `Bash`를 쓴 회차는 없었다. `Read`를 건너뛴 것이 문제였다. |
| 구성 | `git status --short` 한 명령이 읽기 전용 목록을 대표하지 않는다. | 결론을 이 명령 하나로 제한했다. |
| 구성 | `.git` fixture는 실제 Git metadata가 아니다. | `.git` 이름의 폴더 아래 `Write`로 제한했다. |
| 구성 | `flag_deny_rule`은 사용자 설정 위치가 아니다. | 사용자 설정 위치는 측정 불가로 두고 폴더 설정과 flag 설정만 결론냈다. |
| 외적 | 버전, 모델, 도구 이름이 바뀌면 달라진다. | 2.1.288, `haiku`, 2026-10-04에 한정한다. |
| 외적 | `haiku`의 도구 선택이 다른 모델과 다를 수 있다. | `Read` 선행 여부는 모델마다 다를 수 있다. 결론을 `haiku`로 제한했다. |

### 한계

- 사용자 설정 위치(`~/.claude/settings.json`)의 `deny` 규칙과 훅은 측정하지 못했다. 격리용 `CLAUDE_CONFIG_DIR`은 로그인이 없고 새 로그인과 복사는 하지 않았다.
- 판정은 3회씩이라 비율 구간이 넓다. 3/3의 하한은 29.2%다.
- Saturn engine의 실제 규칙 평가, 저장, TUI 전달은 측정하지 않았다.
- 작업 폴더 밖 편집의 확인 분석은 `Read` 요청에 가려 기각으로 남았다. 읽기 지시 재수집이 이를 풀었지만 판정에는 쓰지 않았다.
- 점검 호출 4회의 원문은 저장하지 않았다. 판정 계산에서 쓴 `/tmp` 아래 임시 체크섬 파일이 한 번 생겼고 바로 지웠다.

## 재현

```sh
./run.sh verify
./run.sh analyze
```

| 파일 | SHA-256 |
|---|---|
| `data/SHA256SUMS` | `f770983637220eaa88d0944d0a4aded332322b2eec0aee6bfb15a6d6c30e630a` |

## 결론

| 가설 | 판정 | 반영한 문서 |
|---|---|---|
| H1 | 채택: 요청 3/3, 거부 뒤 marker 0/3 | 없음: 기존 권한 설계의 Saturn 거부 응답 경로를 실측으로 확인했다. |
| H2 | 불채택: `Edit` 2/3(`Read` 누락 1회), `Write` 3/3, 읽기 명령 3/3 | 없음: `Edit` 경계는 읽기 지시 재수집 3/3(탐색)으로만 본다. |
| H3 | 불채택: 밖 편집 0/3, `add-dir` 편집 2/3(`Read` 누락) | 없음: 재수집은 밖 묻기 3/3, `add-dir` 허용 3/3(탐색). |
| H4 | 채택: `.git` 묻기 3/3 | 없음 |
| H5 | 채택: MCP 요청 3/3, `mcp__*` 패턴 통함 | 없음 |
| H6 | 채택: `Agent` 요청 3/3, 하위 `touch` 거부 3/3 | 없음 |
| H7 | 채택: 폴더 `deny`, flag `deny`, 훅 각 3/3 요청 0건(사용자 위치는 측정 불가) | 없음 |
| H8 | 채택: `add-dir` `Read` 요청 0건 3/3, 밖 `Read` 요청 3/3 | 없음 |
