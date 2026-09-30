# Saturn

Codex와 Claude Code를 하나의 대화로 이어 쓰는 터미널 도구

설계 문서는 [docs/README.md](docs/README.md)에 있다.

## 구성

| 경로 | 내용 |
|---|---|
| `saturn-protocol` | engine과 TUI가 주고받는 메시지 타입 정의 |
| `saturn-terminal/core` | 대기열, session, 에이전트 트리, 판단 규칙과 외부 연결 trait |
| `saturn-terminal/engine` | provider 연결, judge 호출, 기록 저장, TUI 접속을 맡는 상주 프로세스 |
| `saturn-terminal/tui` | 채팅 기록, 실행 영역, 상태판, 입력창을 그리는 전체 화면 |
| `saturn-terminal/cli` | `saturn` 실행 파일, 명령줄 처리와 engine 시작 |
| `docs/` | 설계 문서 |

## 규칙

- 동작, 계약, 설정 변경은 같은 PR에서 설계 문서 갱신
- 새 문서는 `docs/README.md` 문서 목록 안에서만 추가
- `README.md`를 고치면 같은 PR에서 `README.ko.md`도 수정
- 입력은 기록 저장소에 접수된 뒤에만 provider로 전송
- 보내기 전에 확정된 실패만 재전송
- 판단 결과는 적용 직전 채팅 revision 비교
- engine은 사용자당 하나, 잠금으로 보호
- 기록 저장소는 파일 하나, 쓰는 쪽은 engine 하나
- `core`는 파일, 네트워크, 프로세스를 직접 다루지 않음
- provider 고유 이름은 `providers/codex`, `providers/claude` 안에서만 사용
- 에이전트끼리 직접 통신 금지
- provider 설정과 subagent 사용은 추적만, 예외는 judge 키 보호
- judge는 engine만 호출, HTTPS만 사용, TLS 검증 유지
- judge 키와 일치하는 문자열은 로그, 오류, 디버그 출력에서 은닉
- 스키마는 새 버전을 처음 실행할 때 자동 이관, 이관 직전 백업 하나를 14일 보관
