# Codex와 Claude는 직접 연결을 유지하고 새 provider는 ACP 어댑터로 시작한다

| 항목 | 값 |
|---|---|
| 날짜 | 2026-10-04 |
| 상태 | 채택 |
| 대체 | 없음 |
| 관련 문서 | [provider 연결과 session](../design/providers-and-sessions.md), [기능 목록과 확장](../design/extensions.md), [아키텍처](../architecture.md) |

## 배경

[열린 id의 어댑터 결정](2026-10-04-open-providers-and-saturn-extensions.md)으로 provider는 어댑터 하나를 더해 붙는다. 어댑터를 engine 프로세스 밖에 두고 표준 메시지로 대화하게 하면 공통 코드를 고치지 않고도 provider를 붙일 수 있다. 후보는 공개 표준 Agent Client Protocol(ACP)이다.

- ACP는 Zed와 JetBrains가 함께 관리한다([관리 문서](https://agentclientprotocol.com/community/governance), 2026-10-04 확인).
- 저장소 라이선스는 Apache-2.0이다([저장소](https://github.com/agentclientprotocol/agent-client-protocol), 2026-10-04 확인).
- 확장은 `_`로 시작하는 메서드 이름과 `_meta` 필드로 한다([확장 문서](https://agentclientprotocol.com/protocol/extensibility), 2026-10-04 확인).

Saturn은 Codex app-server와 Claude stream-json으로 끼워 넣기, 보내기 전 실패와 보낸 뒤 불명의 구분, 하위 에이전트 관리, 맥락 정리 지시, 사용량 상세를 쓴다. 사용자가 결정 때 정리한 바로는 이 다섯은 ACP 안정판에 없다.

## 선택지

| 선택지 | 장점 | 단점 |
|---|---|---|
| Codex와 Claude는 직접 연결을 유지하고, 새 provider는 처음에 ACP 어댑터 하나로 붙이며, 주력이 되고 provider가 깊은 기능을 열면 직접 어댑터로 바꾼다 | 지금 쓰는 깊은 기능 유지, 새 provider를 공통 코드 수정 없이 기본 기능으로 시작, 교체가 어댑터만 바꾸는 일 | 직접 연결의 provider 업데이트 추적 부담, 새 provider는 처음에 깊은 기능 없음 |
| ACP를 Saturn 계약으로 삼고 Codex와 Claude도 ACP로 연결 | 어댑터 형식 하나 | 끼워 넣기, 전달 실패 구분, 하위 에이전트 관리, 맥락 정리 지시, 사용량 상세를 잃음 |
| Saturn 자체 계약을 두고 ACP 어댑터를 지금 구현, 부족한 기능은 ACP 확장으로 보탬 | 모든 provider에 같은 경로 | provider, 원본 번역기, ACP 명세 세 겹을 따라가야 해 유지보수가 더 듦, 아직 ACP로 붙일 provider가 없음 |

## 결정

Codex와 Claude는 지금처럼 직접 연결(Codex app-server, Claude stream-json)을 유지하고 ACP로 연결하지 않는다. 앞으로 새 provider는 처음에 ACP 어댑터 하나로 붙여 기본 기능으로 쓰고, 주력으로 쓰게 되고 그 provider가 깊은 기능을 열어 주면 직접 어댑터로 바꾼다. 세 계층으로 나눠 두었으므로 이 교체는 어댑터만 바꾸는 일이다. ACP 어댑터는 지금 구현하지 않는다. 깊은 기능을 잃지 않는 것과 유지보수 층을 늘리지 않는 것을 가장 중요하게 본다.

직접 연결의 업데이트는 두 가지로 대응한다. engine이 시작할 때 provider CLI 버전을 읽어 마지막으로 확인한 버전과 다르면 알린다. 버전이 바뀌면 실제 provider로 핵심 흐름(입력, 전환, 다시 열기)만 도는 빠른 확인 절차를 `scripts/e2e`에 둔다. 차이는 어댑터 안에서만 고친다.

## 결과

- Codex와 Claude의 깊은 기능 유지
- 새 provider를 어댑터 하나로 붙이는 경로
- 직접 연결의 provider 업데이트 추적과 빠른 확인 절차 유지
- ACP 어댑터가 생기기 전까지 새 provider를 붙일 수 없음

## 다시 볼 조건

- ACP 안정판이 끼워 넣기, 전달 실패 구분, 하위 에이전트 관리, 맥락 정리 지시, 사용량 상세를 담음
- 직접 연결의 업데이트 대응이 어댑터 밖 수정을 계속 요구
- 붙이고 싶은 새 provider가 생겨 ACP 어댑터 구현이 필요
