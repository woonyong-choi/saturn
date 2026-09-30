# engine만 judge를 부르고 자식 프로세스 환경에서 judge 키를 지운다

| 항목 | 값 |
|---|---|
| 날짜 | 2026-09-29 |
| 상태 | 채택 |
| 대체 | 없음 |
| 관련 문서 | [아키텍처](../architecture.md), [judge 키 보호](../design/judge-key-security.md) |

## 배경

judge는 입력마다 뜻을 판단하는 외부 API이고 키가 필요하다. 키를 환경 변수로 주면 provider 자식 프로세스가 같은 환경을 그대로 받는다. provider와 subagent는 judge를 부를 필요가 없다. macOS `security` 명령으로 키체인에 저장하면 그 명령이 신뢰 앱이 된다(2026-09-29 확인). 그러면 다른 프로세스도 같은 명령으로 확인 창 없이 키를 읽을 수 있다(Silverfort 보고).

## 선택지

| 선택지 | 장점 | 단점 |
|---|---|---|
| engine 프록시, 키체인 OS API 저장, 자식 환경의 키 변수 제거 | 에이전트에 키 미전달, 저장 위치 보호 | 제거 목록과 훅 검사 유지 |
| 환경 변수로 키 전달 | 적은 구현량, 별도 저장소 없음 | provider와 subagent에 키 노출 |
| 암호문 파일 저장, 실행마다 암호 확인 | 운영체제와 무관한 동일 동작 | 실행마다 암호 입력 |

## 결정

engine 프록시, 키체인 OS API 저장, 자식 환경의 키 변수 제거를 쓴다. 에이전트가 어떤 경로로도 키를 받지 않는 것을 가장 중요하게 본다.

## 결과

- judge 호출 창구를 engine 하나로 통일
- 자식 환경에 제거 목록의 이름이 없음을 테스트로 고정
- 제거 목록과 훅 검사의 유지 부담

## 다시 볼 조건

- [키체인 직접 읽기 실험](https://github.com/woonyong-choi/saturn/issues/2)에서 확인 창 없이 읽히는 경로 발견
- [훅 키 차단 실험](https://github.com/woonyong-choi/saturn/issues/3)에서 provider 훅으로 차단 불가
