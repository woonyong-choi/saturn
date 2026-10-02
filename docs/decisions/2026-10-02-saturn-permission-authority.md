# 권한 판단의 정본은 Saturn 설정의 `permission` 규칙 하나로 둔다

| 항목 | 값 |
|---|---|
| 날짜 | 2026-10-02 |
| 상태 | 채택 |
| 대체 | 대체한 결정: [provider 설정과 subagent 사용은 사용자 설정을 따르고 Saturn은 추적만 한다](2026-09-29-minimal-provider-control.md)의 권한 부분 |
| 관련 문서 | [권한](../design/permissions.md), [provider 연결과 session](../design/providers-and-sessions.md), [설정](../design/settings.md), [TUI](../design/tui.md), [judge 키 보호](../design/judge-key-security.md) |

## 배경

Saturn은 셸 명령, 파일 편집, MCP 도구, subagent 실행을 사용자에게 묻고 답을 provider에 돌려줘야 한다. 이전 결정은 권한을 포함한 provider 설정을 사용자 설정에 맡기고 Saturn은 추적만 하도록 정했다. 2026-10-02 실측에서 Codex 0.158.0은 사용자나 폴더의 허용 규칙에 걸린 명령을 묻지 않고 실행했다. app-server에는 규칙을 무시하는 인자가 없었다. 전용 `CODEX_HOME`, `thread/start`의 `approvalPolicy="untrusted"`, 읽기 전용 샌드박스를 합치면 셸, 파일 편집, subagent 명령이 승인 요청으로 왔다. Codex PreToolUse 훅은 `ask` 응답을 지원하지 않는다. 훅은 시간 초과와 오류에서 도구를 실행하고, subagent 생성과 파일 편집에는 걸리지 않는다. Claude Code 2.1.285는 `--permission-prompt-tool stdio`와 `--settings`의 `ask` 목록을 함께 주면 `Bash` 호출을 모두 호스트로 보냈다. 사용자 `bypassPermissions`와 폴더 허용 목록이 있어도 같았다. 근거는 [이슈 194](https://github.com/woonyong-choi/saturn/issues/194)와 [실험 보고서 PR 230](https://github.com/woonyong-choi/saturn/pull/230)에 있다.

## 선택지

| 선택지 | 장점 | 단점 |
|---|---|---|
| Saturn 설정의 `permission` 규칙을 정본으로 두고 provider별로 번역 | 두 provider에 같은 규칙, provider 파일 불변, 항상 허용을 Saturn 기록에 보관, 사용자 규칙이 끼어들지 않아 판정 재현 | provider별 구성 구현, Codex 전용 `CODEX_HOME` 매 실행 생성, 읽기 전용 샌드박스 불편 가능 |
| 사용자 설정 우선, Saturn은 추적만 | 사용자 provider 설정 유지, 구현 최소 | Codex 허용 규칙 명령을 묻지 못함, 사용자 설정이 바뀌면 Saturn 판단도 바뀜 |
| Saturn 공통 설정을 provider 설정으로 번역해 사용자 설정과 병합 | 사용자 provider 규칙 유지, 전용 폴더 불필요 | 사용자 허용 규칙이 이겨 Codex에서 묻기 강제 불가, provider마다 우선순위가 달라 결과 예측 어려움 |
| 사용자 provider 설정 파일에 규칙 기록 | provider를 직접 쓸 때도 같은 규칙 | 사용자 파일 변경과 되돌림 부담, 사용자가 정한 provider 설정 덮어쓰기 |

## 결정

Saturn 설정의 `permission` 규칙을 권한의 정본으로 쓴다. 두 provider에서 같은 규칙으로 판단하는 것과 사용자 provider 파일을 고치지 않는 것을 가장 중요하게 본다.

## 결과

- 셸 명령, 파일 편집, MCP 도구, subagent 실행을 `allow`, `ask`, `deny` 규칙 하나로 판단
- 권한 모드(`ask`, `edit`, `read-only`, `full`)를 기본 규칙 묶음으로 두고 개별 규칙을 그 위에 덧붙임
- 사용자와 폴더 어느 층이든 개별 `deny`가 하나라도 일치하면 거부
- 항상 허용을 provider 파일이 아닌 Saturn 기록 저장소에 보관
- 사용자 provider 파일 불변
- 권한 외 provider 설정(모델, MCP 서버 등)과 subagent 사용은 이전 결정대로 사용자 설정을 따르고 추적
- Codex 전용 `CODEX_HOME` 생성과 사용자 권한 설정 무시
- 규칙이 다른 채팅마다 Codex app-server와 `CODEX_HOME` 분리
- Codex 파일 편집까지 승인 요청으로 받는 읽기 전용 샌드박스
- Claude `ask` 목록에 대상 도구 이름 나열

## 다시 볼 조건

- Codex PreToolUse 훅이 `ask`를 지원([openai/codex#28437](https://github.com/openai/codex/issues/28437)이 해결됨)
- Codex나 Claude가 규칙 우선순위를 바꿔 사용자 설정이 Saturn 요청을 다시 우회
- 읽기 전용 샌드박스에서 일반 읽기, 빌드, 테스트 작업이 자주 막힘([이슈 232](https://github.com/woonyong-choi/saturn/issues/232) 측정)
- 키체인 로그인에서 전용 `CODEX_HOME`이 로그인을 공유하지 못함
- Codex app-server가 thread별 execpolicy 선택을 제공
