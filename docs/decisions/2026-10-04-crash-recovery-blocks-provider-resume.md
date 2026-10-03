# 크래시 뒤 provider가 끊긴 작업을 다시 하지 못하게 막는다

| 항목 | 값 |
|---|---|
| 날짜 | 2026-10-04 |
| 상태 | 채택 |
| 대체 | 없음 |
| 관련 문서 | [engine 수명과 복구](../design/engine-lifecycle.md), [provider 연결과 session](../design/providers-and-sessions.md), [강제 종료 뒤 provider session 재개 결과](../experiments/crash-resume/report.md) |

## 배경

크래시 뒤 기록에는 끝 신호 없이 실행 중인 하위 에이전트가 남는다. 증명되지 않은 실행을 자동으로 재개하지 않기로 했는데([결정](2026-09-29-proof-based-auto-resume.md)), provider가 부모 session을 다시 열 때 자식 session을 불러오거나 끊긴 턴을 이어 가면 같은 작업이 사용자 몰래 다시 실행될 수 있다. Codex는 자식 thread 정리 요청(`thread/archive`, `thread/unsubscribe`)이 3/3 성공했고 raw 재개에서도 재실행이 0/3이었다. Claude Code 2.1.288은 `CLAUDE_CODE_RESUME_INTERRUPTED_TURN`이 작업자 재시작으로 끊긴 턴을 자동으로 다시 실행하게 한다고 설치본이 설명하지만, 로그인 만료로 효과는 측정하지 못했다.

## 선택지

| 선택지 | 장점 | 단점 |
|---|---|---|
| 하위 에이전트를 끊김으로 기록하고 제안만, Codex는 재개 전 자식 정리, Claude는 환경 변수 제거, 끊긴 자식이 다시 오면 막기 | 자동 재개 금지가 provider 쪽에서도 지켜짐, provider가 동작을 바꿔도 알아챔 | provider 환경 변수 하나를 지우는 예외, 정리 요청 구현 |
| 끊김 표시와 제안만, provider 재개 동작은 그대로 | provider 설정을 바꾸지 않는 원칙 유지 | provider가 이어 가면 외부 효과가 반복될 수 있음 |

## 결정

첫 선택지를 쓴다. 증명 없이 같은 작업이 다시 실행되지 않게 하는 것을 가장 중요하게 본다.

## 결과

- 크래시 복구는 실행 중으로 남은 하위 에이전트를 끊김으로 기록하고, 다시 할지는 사용자에게 제안만 한다.
- Codex는 부모 thread를 다시 열기 전에 기록한 끊긴 자식 thread만 보관하고 구독을 끊는다. 부모 thread는 건드리지 않는다.
- Claude는 실행 환경에서 `CLAUDE_CODE_RESUME_INTERRUPTED_TURN`만 뺀다. [provider 설정을 바꾸지 않는 원칙](2026-09-29-minimal-provider-control.md)의 예외는 router 키 보호에 이어 이것 하나다.
- 다시 연 뒤 끊긴 하위 에이전트의 이벤트가 오면 기록하지 않고 작업을 멈추며 화면에 알린다.

## 다시 볼 조건

- Claude 로그인이 가능해져 `CLAUDE_CODE_RESUME_INTERRUPTED_TURN`의 효과를 실측했을 때, 변수 제거가 필요 없거나 부족하다고 나오면
- provider 업데이트로 끊긴 자식이 정리 뒤에도 다시 실행되는 일이 관찰되면
