# saturn

Codex와 Claude Code를 하나의 대화로 이어 쓰는 터미널 도구

설계 문서는 [docs/README.md](docs/README.md)에 있다.

## 구성

| 경로 | 내용 |
|---|---|
| `saturn-protocol` | 약속 |
| `saturn-terminal/core` | 핵심 규칙 |
| `saturn-terminal/engine` | 엔진 |
| `saturn-terminal/tui` | 화면 |
| `saturn-terminal/cli` | 명령 |
| `docs/` | 설계 문서 |

## 규칙

- 동작, 계약, 설정 변경은 같은 PR에서 설계 문서 갱신
- 새 문서는 `docs/README.md` 문서 목록 안에서만 추가
- 보내기 전에 확정된 실패만 재전송
- 판단기 키와 일치하는 문자열은 로그, 오류, 디버그 출력에서 은닉
- 엔진은 사용자당 하나, 잠금으로 보호
- 기록 저장소는 파일 하나, 쓰는 쪽은 엔진 하나
- 판단기는 엔진만 호출
- 스키마는 새 버전을 처음 실행할 때 자동 이관, 이관 직전 백업 하나를 14일 보관
