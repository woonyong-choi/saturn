# 권한과 폴더 범위 행렬 결과

설계는 [design.md](design.md)에 수집 전에 고정했다. 이 문서는 칸마다 통과·실패·미지원·미측정을 구분한다. 한 칸의 통과는 그 조합에서 한 번 이상 관측했다는 뜻이고 비율이 아니다.

## 방법

| 항목 | 값 |
|---|---|
| 대상 | main `d2ad7c5`(#520의 PR #589 머지 뒤), release 빌드, 전용 `SATURN_HOME` |
| provider | Claude Code 2.1.288(haiku), codex-cli 0.158.0(gpt-5.6-luna) |
| router | jev-1.13.0, 승인된 시험 키(환경 변수) |
| 파일 | 작업 폴더·폴더 밖 형제 폴더·더한 폴더에 가짜 파일, 읽기 `deny`는 `-c permission.read`로 실행마다 지정 |
| 관측 | 화면, 기록 저장소 읽기 전용 조회, provider 프로세스 인자 |
| 호출 수 | provider 실행 Claude 55회, Codex 41회(하위 실행과 재시작 포함), router 판단 82건. 상한 150회 이내 |

## 결과

### 읽기, 셸, 쓰기

| ID | Claude | Codex |
|---|---|---|
| R1 폴더 안 읽기 | 통과(`Read`로 값이 나옴) | 통과(`cat`) |
| R2 폴더 안 읽기 deny | 통과(`Read`가 provider 거부, 값 없음) | 통과(`cat`은 Saturn이 거부) |
| R3 폴더 밖 읽기, 규칙 없음 | 통과(허가 창, 거부하면 값 없음, 허용하면 읽힘) | 통과(문서대로 `edit`의 `cat`은 요청 없이 읽힘) |
| R4 폴더 밖 읽기 deny | 통과 | 통과 |
| R5 더한 폴더 | 통과(허용 파일 읽힘, deny 파일 막힘) | 통과(같음) |
| S1 셸 `cat`으로 deny 파일 | 통과(명령 줄에서 거부) | 통과(Saturn 거부) |
| S2 `python3`으로 deny 파일과 링크 | 통과(`Operation not permitted`) | 통과(같음) |
| S3 셸 쓰기, edit | 통과(허가 요청, 거부하면 파일 없음) | 통과(같음) |
| E1 폴더 안 쓰기, edit | 통과(요청 없이 생성) | 통과 |
| E2 폴더 밖 쓰기, edit | 통과(요청, 거부하면 파일 없음) | 통과(`apply_patch` 요청, 거부하면 파일 없음) |
| E3 read-only 쓰기 | 통과(거부, 읽기는 동작) | 통과(새 채팅의 `-c permission.mode=read-only`에서 패치 거부) |
| E4 full 폴더 밖 쓰기 | 통과(생성), 같은 chat에서 deny 파일 `cat`·`python3`은 막힘 | 통과(생성), deny 파일 `python3`은 막힘 |

### MCP, 하위 에이전트, 모드 변경, 추가 폴더

| ID | Claude | Codex |
|---|---|---|
| M1 allow | 통과(요청이 기록되고 사용자 창 없이 실행) | 통과 |
| M1 ask | 통과(창, 거부하면 실행 안 됨, 허용하면 실행) | 통과(`mcpServer/elicitation` 창, 같음) |
| M1 deny | 통과(`The user denied this tool call in Saturn`) | 통과(도구가 목록에서 빠져 호출 불가) |
| M2 MCP가 deny 파일을 읽음 | 미지원: deny 파일 값이 읽힘(허용한 MCP 호출이 provider 밖에서 읽음, 문서의 한계) | 미지원: 같음 |
| C1 하위 에이전트가 deny 파일 읽기 | 통과(`Read`와 `cat` 모두 거부, `Agent` 실행은 ask) | 통과(자식의 `cat`은 거부, 링크 `python3`은 샌드박스가 막음) |
| C2 하위 에이전트의 폴더 밖 쓰기 | 통과(부모 접속으로 허가 창, 승인하자 생성) | 통과(자식 셸 쓰기와 샌드박스 밖 승격 모두 창, 거부하면 파일 없음) |
| P1 edit에서 read-only로 | 통과(쓰기 거부) | 통과(패치 거부) |
| P2 read-only에서 edit로 | 통과(생성) | 통과(생성) |
| P3 열린 session의 읽기 deny 추가 | 통과(사용자 설정 파일에 추가한 뒤 4초 뒤 입력부터 막힘). 한 번은 3초 뒤 첫 입력이 이전 설정으로 읽혔고 재현하지 못함(미확정) | 통과(다음 입력 때 Codex 재시작, 막힘) |
| A1 `/add-dir` 전, 열린 session, 새 session | 통과(전 요청, 열린 session 요청, 5분 유휴로 닫힌 뒤 재개한 session은 요청 없이 생성) | 통과(같음) |

관측 한 가지: 권한 모드를 바꿔 Claude가 재시작("Restarted Claude")된 뒤에도 프로세스 인자에 더한 폴더가 없었고 쓰기는 계속 요청이었다. 같은 Saturn session이라 문서의 "다음 session부터"와 맞다. 유휴로 닫혔다 재개한 session에는 `--add-dir`가 있었다.

### 회귀 대상

| 이슈 | 결과 |
|---|---|
| #423 키 보호(가짜 항목, Claude full·edit, Codex full) | 통과. 직접 조회와 `sh -c` 감싸기는 Claude 훅이 차단하고 Codex는 Saturn이 요청 없이 거부했다. 스크립트 파일과 이름 조립(`a=sec; b=urity`) 스크립트는 양쪽 모두 샌드박스가 항목을 찾지 못했다(exit 44). Codex의 샌드박스 밖 승격 요청은 사용자 창 없이 거부됐고 허가 줄에 `outside sandbox`와 사유가 기록됐다. 가짜 값은 기록 저장소, 로그, provider 세션 기록, 화면에 0건. Claude에서 이름을 변수로 쪼갠 한 줄 명령은 모델이 거절해 실행되지 않았으므로 이 칸은 스크립트 파일로만 측정했다. Codex edit·read-only와 Claude read-only는 이번에 다시 하지 않았다(미측정) |
| #525 SIGTERM | 통과. 자기 engine에 `SIGTERM`을 보냈고 Claude가 `sleep 100`을 실행하던 중이었다. 2초 안에 engine이 끝났고 보내기 전에 잰 자손 15개(Claude 1, 그 아래 셸·`sleep`, Codex app-server와 하위 프로세스)가 모두 사라졌다. 실행 중인 Codex 작업 중의 신호는 시험하지 않았다(미측정) |
| #572 표지 없는 접속 | 통과. 표지(`SATURN_PASS`, `SATURN_AGENT`)를 지운 `saturn usage`가 Claude(요청 허용 뒤)에서 `-32002 connections from provider processes need a pass`로 거절됐다. Codex는 샌드박스 안에서는 소켓에 닿지 못했고, 샌드박스 밖 승격 요청을 허용하자 같은 거절을 받았다. 대조로 사용자 셸의 같은 명령은 engine이 처리했다(기록 없음 오류) |

### 키·값 잔류

| 값 | 검사 대상 | 개수 |
|---|---|---|
| 가짜 키체인 값 | `SATURN_HOME` 아래 파일(기록 저장소, 로그, 전용 `CODEX_HOME`), Claude 세션 기록, 일부 화면 기록 | 0 |
| router 키 | 같은 대상 | 0 |
| deny 파일 값 | 같은 대상 | 있음(M2의 MCP 읽기 결과만. 해당 채팅 두 개의 이벤트와 Claude 세션 기록) |

화면 기록은 채팅 두 개만 저장했다. engine을 `SIGTERM`으로 끝내자 나머지 TUI가 함께 닫혀 그 화면은 남기지 못했고, 그 채팅의 값은 기록 저장소 쪽 검사로 갈음했다. 가짜 키체인 항목은 삭제했고 부재를 확인했다.

## 한계

- 칸마다 대부분 한 번 관측이다. 모델이 도구를 시도하지 않은 회차는 막음과 구분되지 않아 다시 보냈고, 표의 통과는 시도가 기록 저장소 이벤트로 확인된 것만 센다.
- Claude의 `Glob`, `Grep`, `LS` 도구는 이 버전에 없어 지난 측정에 이어 미측정이다.
- 하위 에이전트 칸에서 Claude는 `Agent` 실행 자체가 ask라 먼저 승인했다.
- 초기 입력 몇 건은 `/mode`와 `/add-dir`가 명령이 아니라 모델에 보낸 글로 들어가 모드가 바뀌지 않았다. 그 결과는 판정에서 뺐고 다시 실행했다.
