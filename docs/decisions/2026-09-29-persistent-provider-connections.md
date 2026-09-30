# provider마다 입력을 계속 받는 상시 연결을 만든다

| 항목 | 값 |
|---|---|
| 날짜 | 2026-09-29 |
| 상태 | 채택 |
| 대체 | 없음 |
| 관련 문서 | [아키텍처](../architecture.md), [provider 연결과 session](../design/providers-and-sessions.md), [입력 처리](../design/input-handling.md) |

## 배경

실행 중 들어온 새 입력을 진행 중인 턴에 바로 반영해야 한다. 한 번 실행 방식인 `codex exec`와 `claude -p`는 실행 중 입력을 받지 못한다. 공식 문서 기준 Codex의 끼워 넣기는 app-server의 `turn/steer`로 한다(2026-09-29 확인). Claude는 stream-json 입력으로 추가 메시지와 멈춤 신호를 받는다(2026-09-29 확인). `codex exec` 이벤트는 session 누적 사용량만 주고 창 크기와 압축 이벤트를 주지 않는다(2026-09-29 확인). Codex app-server는 `tokenUsage`로 마지막 사용량과 맥락 창 크기를 알린다(2026-09-29 확인).

## 선택지

| 선택지 | 장점 | 단점 |
|---|---|---|
| 상시 연결(Codex app-server, Claude stream-json 입력) | 끼워 넣기와 멈춤 신호, 턴마다 맥락 크기 관찰, session 안 맥락 정리(compaction) 요청 | provider별 연결 규약 구현과 유지 |
| 한 번 실행 방식(`codex exec`, `claude -p`) | 턴과 같은 프로세스 수명, 적은 구현량 | 끼워 넣기 불가, Codex 맥락 크기 관찰 불가, subagent 정보 부족 |

## 결정

상시 연결(Codex app-server, Claude stream-json 입력)을 쓴다. 실행 중 입력의 바로 반영, 맥락 크기 관찰을 가장 중요하게 본다.

## 결과

- 끼워 넣기, 멈춤 신호, 맥락 크기 관찰 지원
- provider 연결 규약 변경의 계속 추적
- 끼워 넣기 실측을 통과하기 전의 provider는 끼워 넣기를 대기로 대체

## 다시 볼 조건

- [끼워 넣기 경로 실험](https://github.com/woonyong-choi/saturn/issues/5)에서 문서와 다른 동작 확인
- 한 번 실행 방식의 실행 중 입력 지원
