# provider 설정과 subagent 사용은 사용자 설정을 따르고 Saturn은 추적만 한다

| 항목 | 값 |
|---|---|
| 날짜 | 2026-09-29 |
| 상태 | 채택 |
| 대체 | 없음 |
| 관련 문서 | [아키텍처](../architecture.md), [provider 연결과 session](../design/providers-and-sessions.md), [설정](../design/settings.md) |

## 배경

Codex와 Claude는 스스로 subagent를 띄울 수 있다(2026-09-29 확인). subagent 수, 백그라운드 실행, 샌드박스, 네트워크, 권한은 provider 설정으로 정해진다. Saturn이 실행 인자로 이 값을 고정하면 사용자가 provider에 정한 설정과 달라진다. 멈춤, 쓰기 잠금, 맥락 정리(compaction) 경계는 subagent까지 끝났는지 알아야 판정할 수 있다.

## 선택지

| 선택지 | 장점 | 단점 |
|---|---|---|
| 사용자 설정 우선, Saturn은 추적만 | 사용자가 정한 provider 동작 유지, provider 기능 추가에 대응 불필요 | subagent 추적 구현, 설정으로 증명하지 못한 실행의 자동 재개 불가 |
| 실행 인자로 subagent와 네트워크 고정 | 예측 가능한 실행 범위, 자동 재개 증명의 단순화 | 사용자 설정 무시, provider 기능 제한 |

## 결정

사용자 설정 우선, Saturn은 추적만 하는 방식을 쓴다. 사용자가 고른 provider 동작의 유지를 가장 중요하게 본다.

## 결과

- Saturn 기본값은 사용자 설정이 없을 때만 적용
- subagent의 시작, 진행, 끝 기록
- 작업 끝을 트리 유휴로 판정
- judge 키 보안(환경 변수 제거, 훅)만 예외
- 설정으로 증명하지 못한 실행의 자동 재개 불가

## 다시 볼 조건

- [Claude subagent 이벤트 실험](https://github.com/woonyong-choi/saturn/issues/17)에서 subagent의 끝 확인 불가
- [Codex 자식 session 실험](https://github.com/woonyong-choi/saturn/issues/20)에서 자식 session의 끝 확인 불가
