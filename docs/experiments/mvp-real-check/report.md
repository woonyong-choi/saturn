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

## 3차 수정 재확인

main `b83ffa3`(PR #510~#514 머지 상태)에서 Saturn이 띄운 실제 Codex 0.158.0(gpt-5.6-luna)과 Claude Code 2.1.288(haiku)로 다시 확인했다. router 키가 없어 `router.skip_check`로 돌렸고 router 판단은 확인하지 못했다. 가짜 키체인 항목만 쓰고 끝난 뒤 지웠다. 기록 저장소 조회와 화면 캡처로 관측했고, Codex 요청 필드는 Saturn이 띄운 `codex app-server`의 입출력을 시험용 래퍼로 남겨 필드 이름만 확인했다. provider 실행은 약 25회(Codex 14, Claude 11)다.

| 이슈 | 확인 | 결과 |
|---|---|---|
| #508 | Claude edit에서 Bash 허가 창, 거부하면 파일 없음. read-only에서 `touch`는 창 없이 거부되고 파일 없음. full에서 가짜 키 조회는 훅이 차단 | 통과 |
| #423 | Codex edit, read-only, full에서 샌드박스 밖 직접 조회가 키 판정으로 거부되고 허가 줄에 `outside sandbox`가 표시됨. 가짜 값은 출력, 기록, 로그, 화면에 없음 | 통과 |
| #423 | Claude 프로젝트 로컬 `sandbox.excludedCommands`가 있으면 session을 열지 않고 파일 경로와 이유를 보임 | 통과 |
| #423 | Claude 사용자 설정 층의 제외 목록, Codex `additionalPermissions`와 `networkApprovalContext`가 실제로 오는지, Claude edit와 read-only의 감싸기·스크립트·이름 조립 | 미검증 |
| #507 | Codex edit와 full에서 허가한 `cargo build --offline` 성공, read-only에서 작업 폴더 읽기 허용, 파일 패치는 허가 요청으로 옴 | 통과 |
| #505 | Claude 백그라운드 subagent 실행 중 `/stop`이 2회 모두 20초 안에 실행을 `Stopped`로 닫고, 뒤 입력은 `Held`로 풀림 | 통과 |
| #506 | router 판단 없이 `model.default=codex/...`의 첫 입력이 오토와 매뉴얼 모두 Codex로 열림 | 통과 |

가정 판정: 3차 수정은 `reason`, `additionalPermissions`, `networkApprovalContext`를 샌드박스 밖 신호로 가정했다. 실제 샌드박스 밖 요청(`sandbox_permissions=require_escalated`)에는 비어 있지 않은 `reason`이 붙었고, 일반 승인 요청 7건에는 `reason` 키가 없었다. 나머지 두 필드는 어느 요청에도 오지 않아 판정하지 못했다. 실제 `thread/start`는 `approvalPolicy=untrusted`와 `sandbox=workspace-write`였다.

`--plain` 허가 창은 재현되지 않았다. 창은 화면 맨 위에 그려지고 아래에 채팅 기록이 이어져 화면 뒷부분만 보면 안 보인다.

참고: read-only의 Claude는 `cat` 같은 읽기 셸도 거부한다(셸은 `deny`, 읽기 분류는 Codex에만 있음).
