# MVP 실제 provider 확인 결과

## 요약

main `1263a7c`에서 Saturn이 띄운 실제 Codex 0.158.0과 Claude Code 2.1.288로 전환, 권한, 하위 에이전트, 확장 주입을 확인했다. 이 환경에는 router 키(`SATURN_KEY`)가 없어 router 판단은 확인하지 못했고, router 실패 시 판단 없이 진행하는 경로(`router.skip_check`)만 돌렸다. 아래 결과는 router 검증이 아니다. 새 결함 4건(#505, #506, #507, #508)을 찾았고 #423의 Codex 샌드박스 밖 실행 조건은 실패했다.

## 방법

| 항목 | 값 |
|---|---|
| 대상 | main `1263a7c`, `saturn --plain` |
| provider | Codex 0.158.0(gpt-5.6-luna), Claude Code 2.1.288(haiku, 일부 기본 모델) |
| router | 사용하지 않음. 판단 기록 28건은 모두 `NoResponse` |
| 관측 | 화면 기록, 기록 저장소 읽기 전용 조회 |
| 키 | 가짜 키체인 항목만 사용하고 끝난 뒤 삭제 |
| 호출 수 | provider 실행 28회 이상(하위 실행 포함), router 호출 0회 |

## 결과

| 항목 | 결과 |
|---|---|
| Codex, Claude, Codex 전환과 앞 답 이어짐 | 통과(이어짐은 최근 입력으로도 설명되어 제약 인계의 증거가 아니다) |
| TUI를 닫고 `--continue`로 다시 붙기, 같은 입력 재실행 | 통과(실행 수 변화 없음) |
| 해설과 최종 답 사이 빈 줄(#391) | 통과 |
| 권한 6조합의 키 조회 4종 | Codex는 샌드박스가, Claude는 훅과 샌드박스가 막았다. 값은 나오지 않았다 |
| 권한 6조합의 작업 폴더 읽기·편집·빌드 | 실패 조합 있음(#507, #508) |
| Codex 샌드박스 밖 실행 요청의 키 보호(#423) | 실패. edit와 full 모두 가짜 값이 출력과 기록에 남았다 |
| Claude `excludedCommands` 변형 | 미검증. 프로젝트 층 설정으로는 제외가 적용되지 않았다 |
| 백그라운드 subagent 종료와 쓰기 잠금(#424) | 통과 |
| `/stop` 뒤 정리(#424, #438) | 자식은 정리되나 Claude는 실행이 닫히지 않았다(#505), Codex는 약 10초 넘게 지연 |
| Codex 자식 등록(#438) | 시작과 끝이 한 번씩 기록됨. 등록 전 `thread/closed`와 승인 순서는 관측 경로가 없어 미검증 |
| 확장 주입(#412) | Codex와 Claude 모두 스킬과 MCP 사용 확인 |
| 끼워 넣기(#5) | 미검증. router 없이는 대기로 처리됨 |
| 오래된 제약 인계(#296 조건 2) | 미검증. router 판단 필요 |

## 미검증과 필요한 준비

| 조합 | 필요한 준비 |
|---|---|
| router 판단을 켠 자동 판단, 끼워 넣기, 제약 인계 | 승인된 `SATURN_KEY`를 환경에 둔 세션 |
| Claude `excludedCommands` 적용 상태의 키 보호 | 운영 설정을 건드리지 않는 제외 목록 주입 |
| 하위 에이전트 훅 범위(#23) | 하위에서 가짜 항목 조회를 시키는 실행 |
| OS API로 만든 키체인 항목(#2) | API로 만든 시험 항목 |

## 새로 찾은 결함

- [#505](https://github.com/woonyong-choi/saturn/issues/505): Claude 백그라운드 하위 에이전트를 멈춘 뒤 실행이 끝나지 않고 입력이 대기에 남음
- [#506](https://github.com/woonyong-choi/saturn/issues/506): router 판단이 없을 때 기본 모델을 무시하고 Claude로 감
- [#507](https://github.com/woonyong-choi/saturn/issues/507): Codex 허가 뒤 빌드가 샌드박스에 막히고 read-only에서 읽기가 거부됨
- [#508](https://github.com/woonyong-choi/saturn/issues/508): Claude 셸 명령이 Saturn 허가 판정을 거치지 않음
