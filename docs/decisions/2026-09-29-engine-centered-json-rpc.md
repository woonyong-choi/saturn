# engine을 상주 프로세스로 두고 TUI는 JSON-RPC로 붙는 클라이언트로 만든다

| 항목 | 값 |
|---|---|
| 날짜 | 2026-09-29 |
| 상태 | 채택 |
| 대체 | 없음 |
| 관련 문서 | [아키텍처](../architecture.md), [engine 수명과 복구](../design/engine-lifecycle.md), [TUI](../design/tui.md) |

## 배경

사용자가 TUI를 꺼도 작업은 계속되어야 한다. engine은 provider 연결, 대기열, router, 기록을 맡는다. engine에 붙는 쪽은 TUI(`tui`), `cli`, 나중의 데스크톱 앱으로 여럿이다. 기록 저장소는 SQLite 파일 하나에 쓰는 프로세스를 하나로 두는 규칙을 따른다. Codex TUI도 app-server에 붙는 클라이언트로 동작한다(2026-09-29 확인).

## 선택지

| 선택지 | 장점 | 단점 |
|---|---|---|
| 상주 engine과 Unix 소켓 JSON-RPC | TUI 종료 뒤 작업 계속, 여러 TUI 동시 접속, 사용자당 engine 하나로 쓰기 프로세스 하나 유지 | engine 잠금, 소켓 연결, `protocol` 버전 관리 |
| TUI 프로세스 안에 engine 포함 | 프로세스 하나, `protocol` 정의 불필요 | TUI 종료 때 작업 중지, TUI마다 provider 연결과 기록 쓰기 중복 |

## 결정

상주 engine과 Unix 소켓 JSON-RPC를 쓴다. TUI를 끈 뒤의 작업 계속, 여러 TUI의 동시 접속을 가장 중요하게 본다.

## 결과

- TUI를 꺼도 접수된 대기 입력의 순서대로 처리
- TUI와 앱은 provider를 모르고 Saturn 용어만으로 표시
- `saturn-tui`와 `saturn-cli`의 engine 비의존을 Cargo로 강제
- engine 수명 관리(사용자당 잠금, 유예 뒤 종료) 부담
- engine RPC 메서드 정의 작업 추가

## 다시 볼 조건

- 지원 운영체제에서 Unix 소켓 사용 불가
- 여러 TUI 동시 접속을 쓰는 사용 사례 없음
