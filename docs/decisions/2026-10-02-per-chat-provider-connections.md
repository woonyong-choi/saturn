# provider 연결을 채팅마다 따로 둔다

| 항목 | 값 |
|---|---|
| 날짜 | 2026-10-02 |
| 상태 | 채택 |
| 대체 | 대체한 결정: [provider마다 입력을 계속 받는 상시 연결을 만든다](2026-09-29-persistent-provider-connections.md)의 연결 단위 부분 |
| 관련 문서 | [provider 연결과 session](../design/providers-and-sessions.md), [권한](../design/permissions.md), [아키텍처](../architecture.md) |

## 배경

이전 결정은 provider마다 입력을 계속 받는 상시 연결을 두기로 했지만 연결 하나가 몇 채팅을 맡는지는 정하지 않았다. 채팅은 처음 만든 폴더에 묶이고, provider 실행 환경(PATH 등)은 그 채팅에 가장 최근에 붙은 TUI의 값을 쓴다([이슈 182](https://github.com/woonyong-choi/saturn/issues/182)). 채팅마다 실행 환경과 작업 폴더가 다르므로 한 연결이 여러 채팅을 맡으면 한 채팅의 값이 다른 채팅에 섞인다. 권한은 채팅별 규칙으로 판단한다([결정 기록](2026-10-02-saturn-permission-authority.md)). Codex app-server는 thread별로 execpolicy 파일을 고르게 하는 인자가 없어서 규칙이 다른 채팅은 app-server와 `CODEX_HOME`을 따로 필요로 한다(2026-10-02 확인, codex-cli 0.158.0, [이슈 194](https://github.com/woonyong-choi/saturn/issues/194), [이슈 233](https://github.com/woonyong-choi/saturn/issues/233)).

## 선택지

| 선택지 | 장점 | 단점 |
|---|---|---|
| 채팅마다 provider 연결 | 채팅별 환경, 작업 폴더, 권한 규칙의 격리, 연결 키 하나로 이벤트와 채팅 대응, 채팅 하나의 연결 문제가 다른 채팅에 불전파 | 채팅 수만큼 프로세스 증가 |
| 사용자당 provider 하나 | 최소 프로세스 수, 구현 단순 | 채팅별 환경과 작업 폴더 고정 불가, 규칙이 다른 채팅의 Codex 권한 분리 불가 |
| 규칙이 같은 채팅끼리 공유 | 프로세스 수 감소 | 환경과 작업 폴더가 다르면 공유 불가, 규칙 변경 때 연결 재배정, 공유 단위 관리 구현 |

## 결정

provider 연결(Codex app-server, Claude 프로세스)을 채팅마다 따로 둔다. 채팅별 실행 환경과 작업 폴더, 채팅별 권한 규칙의 격리를 가장 중요하게 본다.

## 결과

- 입력을 계속 받는 상시 연결 성격 유지
- 채팅별 실행 환경과 작업 폴더 고정
- 규칙이 다른 채팅마다 Codex app-server와 `CODEX_HOME` 분리
- 연결 키를 `(채팅, provider)`로 두어 이벤트를 채팅에 바로 대응
- 채팅 수만큼 provider 프로세스 증가
- 유휴 연결 정리는 기존 5분 유예 규칙을 그대로 적용

## 다시 볼 조건

- 동시에 열린 채팅 수가 늘어 provider 프로세스 수가 메모리나 시작 시간 문제로 드러남
- Codex app-server가 thread별 execpolicy 선택과 환경 지정을 제공
