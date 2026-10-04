# provider는 열린 id의 어댑터로 붙이고 확장은 Saturn 저장소에 설치해 session을 열 때 주입한다

| 항목 | 값 |
|---|---|
| 날짜 | 2026-10-04 |
| 상태 | 채택 |
| 대체 | 없음 |
| 관련 문서 | [아키텍처](../architecture.md), [provider 연결과 session](../design/providers-and-sessions.md), [기능 목록과 확장](../design/extensions.md), [설정](../design/settings.md), [권한](../design/permissions.md), [기록 저장과 보존](../design/records.md) |

## 배경

Saturn은 Codex와 Claude Code 두 provider를 한 채팅으로 잇는다. 설계는 provider 고유 이름을 `providers/codex`, `providers/claude` 안에서만 쓴다고 정하지만, main 67a6dad에서는 `Provider`가 닫힌 enum이고 어댑터 등록이 정적 `match`이며 어댑터 밖 engine 파일 여럿과 TUI가 provider 이름으로 분기한다. `ProviderClient` trait과 `ProviderEvent`는 provider 중립이다. engine이 provider 명령 목록을 TUI에 보내는 코드는 없고, 확장이라는 개념이 없으며, `PermissionTool`은 닫힌 다섯 종류다. 사용자 결정은 어떤 provider든 Saturn 공통 코드를 고치지 않고 붙이는 것과, 사용자가 설치한 확장이 provider를 바꿔도 쓰이는 것이다. 사용자 provider 설정 파일을 고치지 않는 기존 원칙([권한 결정](2026-10-02-saturn-permission-authority.md))은 유지한다.

## 선택지

| 선택지 | 장점 | 단점 |
|---|---|---|
| Saturn 확장 저장소에 설치하고 session을 열 때 어댑터가 provider 형식으로 주입 | 전환해도 같은 확장 사용, 사용자 provider 파일 불변, 설치 상태를 Saturn 기록에 보관, 옮길 수 없는 부분의 판정과 알림 | 어댑터마다 번역 구현, provider의 확장 형식 변화 추적 |
| provider에만 설치 | 구현이 거의 없음, provider를 직접 쓸 때도 같은 확장 | 전환하면 확장이 사라짐, Saturn이 설치 상태를 알 수 없음, provider마다 따로 설치 |
| 설치 때 사용자가 Saturn과 provider 중 선택 | 확장마다 위치 선택 | 설치마다 질문, 확장의 위치와 상태가 두 곳으로 갈라짐, 전환 때 사라지는 확장을 기억해야 하는 부담 |

## 결정

Saturn 확장 저장소에 설치하고 session을 열 때 어댑터가 provider 형식으로 주입하는 방식을 쓴다. provider는 닫힌 enum 대신 열린 id로 식별하고, 어댑터를 레지스트리에 등록해 어댑터 밖 공통 코드가 provider 이름으로 분기하지 않게 한다. 어댑터 구현이 늘어나는 비용을 치르더라도 provider를 바꿔도 같은 확장이 쓰이는 것과 공통 코드를 고치지 않고 provider를 붙이는 것을 가장 중요하게 본다. 이 결정은 사용자가 2026-10-04에 정했다.

## 결과

- Saturn 인터페이스, 어댑터, 기능 전달 세 계층으로 provider 연결을 나눔
- provider id, 표시명, 실행 파일, 기본 순서, 기능, 지시 문서 이름을 어댑터 설명자가 알림
- provider별 설정 키를 `provider.<id>.*` 열린 이름공간으로 이동
- 기록 저장소의 provider 값과 `<provider>/<model>` 모델 고정 글은 옛 값 그대로 읽힘
- 확장 원본을 `~/.saturn/extensions/`에 두고 부분마다 provider별 사용 가능 여부 판정
- 옮길 수 없는 부분을 설치 때와 전환 때 대화 기록에 한 줄로 알림
- 어댑터별 번역 구현과 provider 확장 형식 변화 추적
- 별도 프로세스 어댑터는 조사 뒤에 정함

## 다시 볼 조건

- 어댑터를 구현하는 데 공통 코드 수정이 계속 필요
- 주입한 확장이 provider에서 사용자가 직접 설치한 같은 확장과 다르게 동작한다는 보고가 반복
- provider가 사용자 설정을 건드리지 않고 확장을 주입하는 통로를 제공하지 않음
- 별도 프로세스 어댑터 조사에서 공개 표준으로 어댑터를 붙일 수 있음이 확인
