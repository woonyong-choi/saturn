# TUI

| 항목 | 값 |
|---|---|
| 상태 | 결정 |
| 관련 결정 | [engine을 상주 프로세스로 두고 TUI는 JSON-RPC로 붙는 클라이언트로 만든다](../decisions/2026-09-29-engine-centered-json-rpc.md), [TUI를 포함한 모든 구성 요소를 Rust로 만든다](../decisions/2026-09-29-rust-for-all-components.md) |

## 요약

TUI는 채팅 기록, 작업별 출력, 상태판, 입력창을 한 터미널에 그리는 전체 화면이다. TUI는 `engine`에 붙는 클라이언트이고 provider를 몰라도 Saturn 용어만으로 화면을 그린다. 여러 작업이 동시에 돌 때 작업마다 이름표를 붙이고, 입력마다 판단, 대기, 보류, 전달 상태를 상태판에 한 줄씩 그린다.

## 동기

Codex와 Claude Code를 함께 쓰는 개발자는 한 저장소에서 여러 작업을 동시에 진행한다. 실행 중에 새 입력을 넣으면 그 입력이 하던 작업에 끼워졌는지, 새 작업이 됐는지, 차례를 기다리는지 보여야 한다. 이것이 보이지 않으면 사용자는 입력이 에이전트에 닿았는지 모른 채 같은 말을 다시 보내게 된다. 스크립트와 CI 사용자는 화면 없이 파이프로 입력을 보내고 결과를 받으므로, 같은 명령이 전체 화면 없이도 같은 결과를 내야 한다.

## 예시

### 실행 중 두 번째 입력이 대기 줄에 뜬다

1. 사용자가 `로그인할 때 세션이 바로 끊기는 버그 고쳐`를 입력하자 작업 A가 시작된다.
2. 작업이 하나뿐이라 대화 기록에는 이름표 없이 입력 에코가 찍힌다.
3. A가 실행 중일 때 사용자가 `테스트도 같이 돌려줘`를 입력한다.
4. 상태판에 `⠹ [C] 판단 중` 줄이 뜨고 router가 입력을 판단한다.
5. router가 A 다음에 보내기로 정하면 줄이 `· [C] 대기 · A 다음`으로 바뀌고 `[보내기] [취소]` 버튼이 붙는다.
6. 살아 있는 작업과 대기 줄이 생겼으므로 A의 줄과 기록에도 이름표 `[A]`가 붙는다.

### 맥락 정리가 기록에 한 줄로 남는다

1. 작업 A의 맥락이 기준에 닿으면 실행 줄의 하는 일이 `맥락 정리 중`으로 바뀐다.
2. 정리가 끝나면 대화 기록에 `[A] 맥락 정리 후 이어서 진행` 한 줄이 남는다.
3. 바닥줄의 `맥락 38K/200K`는 턴마다 새 값으로 바뀐다.

### 멈춘 작업을 다시 연다

1. 사용자가 실행 중에 `Ctrl+C`를 누르자 모든 작업이 멈추고 상태판에 `‖ 멈춤 · [A] [C] 보류됨 · /continue 로 이어서`가 뜬다.
2. 사용자가 TUI를 닫았다가 같은 채팅을 다시 연다.
3. TUI는 보류 재개 질문을 한 번 띄우고 `모두 이어서`, `골라서 이어서`, `그대로 두기` 중 하나를 묻는다.

## 상세 설계

### 배치

TUI는 위에서 아래로 대화 기록, 작업별 출력 칸, 상태판, 팝업, 입력창, 바닥줄을 쌓는다. 아래 그림은 이 배치의 설계다.

```text
┌────────────────────────────────────────────────────────────────┐
│ 대화 기록                                                      │
│ > [A] 로그인할 때 세션이 바로 끊기는 버그 고쳐                 │
│ > [B] README 오타도 고쳐                                       │
│ [B] claude · 8초 · Token 412                                   │
│ [A] 맥락 정리 후 이어서 진행                                   │
└────────────────────────────────────────────────────────────────┘
┌────────────────────────────────────────────────────────────────┐
│ 작업별 출력 칸                                                 │
│ [A] 출력 중인 작업의 최근 줄                                   │
└────────────────────────────────────────────────────────────────┘
┌────────────────────────────────────────────────────────────────┐
│ 상태판                                                         │
│ ⠙ [A] 작업 중                                                  │
│ ⠹ [D] 판단 중 · 배포 스크립트 정리                             │
│ · [C] 대기 · A 다음 · 테스트도 같이 돌려줘  [보내기] [취소]    │
│ ‖ [E] 보류 · codex · /continue E            [이어서] [취소]    │
│ 자동 판단 일시 중단                                            │
└────────────────────────────────────────────────────────────────┘
┌────────────────────────────────────────────────────────────────┐
│ 팝업                                                           │
│ › help        도움말                               Saturn      │
└────────────────────────────────────────────────────────────────┘
┌────────────────────────────────────────────────────────────────┐
│ 입력창                                                         │
│ ›                                                              │
└────────────────────────────────────────────────────────────────┘
┌────────────────────────────────────────────────────────────────┐
│ 바닥줄                                                         │
│ /help 도움말 · Ctrl+C 멈춤                 맥락 38K/200K       │
└────────────────────────────────────────────────────────────────┘
```

### 영역

| 영역 | 보이는 것 | 갱신 시점 |
|---|---|---|
| 대화 기록 | 입력 에코 `> [A] 원문`, 도구 셀, 결과 줄, 맥락 정리와 provider 전환 한 줄, 피드백 질문, 이번 요청 합계 | 판단 확정, 작업 종료, 다시 실행할 때 기록 저장소에서 최근 부분부터 로드, 위로 스크롤할 때 이전 부분 로드 |
| 작업별 출력 칸 | 출력이 흐르는 작업마다 최근 줄, 높이 상한 8줄, 화면이 작으면 상한 축소 | 완성된 줄 단위 갱신, 출력 시작 때 한 번 이동, 작업 종료 때 전체 내용을 대화 기록으로 옮기고 삭제 |
| 상태판 | 실행 줄, 판단 줄, 학습 줄, 대기 줄, 보류 줄, 알림 줄 | `engine` 상태 변경 |
| 팝업 | `/` 명령 목록과 값 목록, `@` 파일 목록, `$` 스킬 목록, 명령 목록 최대 8행과 오른쪽 출처 표시 | 글자 입력마다 목록 필터 |
| 입력창 | `›` 접두 초안, 붙여넣은 내용 요소 | 키 입력 |
| 바닥줄 | 키 안내, 맥락 크기 | 상태 변경, 턴마다 맥락 크기 |
| 시작 화면 | 로고, Saturn 버전, provider 버전, router와 router 버전, 채팅 기본 폴더와 더한 폴더(다른 폴더의 채팅을 이어 열었으면 그 폴더를 보임) | 실행 때, 첫 결과가 오면 대화 기록 맨 위 머리 셀로 전환 |
| router 키 입력 창 | router 확인 실패 원인, 가린 키 입력칸 | 시작 때 router 확인 실패 |
| 폴더 설정 신뢰 창 | 폴더 설정 파일 경로, 지문, 적용되는 항목, 무시되는 항목, 바뀐 줄 | 처음 보거나 내용이 바뀐 폴더 설정을 만난 때, 실행 중이면 engine이 변경을 알아챈 때나 다음 입력 접수 전 |
| 종료 확인 창 | 실행 중인 작업 수, 선택지 `계속 실행`과 `멈추기` | `on_exit`가 `ask`이고 다른 TUI 없이 작업이 남은 채 닫으려 할 때 |
| 멈춤 확인 창 | 끼워 넣기를 받지 않은 충돌 입력의 원문, 질문 `지금 멈추고 새 입력을 실행할까요?`, 선택지 `대기`와 `멈추고 실행` | engine이 `ConfirmStop` 사유의 대기 입력을 알릴 때 |
| 보류 재개 질문 | 보류된 작업 목록, 선택지 `모두 이어서`, `골라서 이어서`, `그대로 두기` | 보류 작업이 있는 채팅을 다시 열 때 한 번 |
| 제약 확인 창 | 확인 종류별 질문(`이 말을 앞으로 지킬 제약으로 등록할까요?`, `새 제약이 앞 제약을 대체할까요?`, `이 제약을 해제할까요?`), 대상 규칙 한 줄(대체는 앞 제약과 새 제약), 선택지 두 개, 답을 기다리는 다른 확인 수. 첫 선택은 지키는 쪽(`등록`, `둘 다 유지`, `유지`)이고 `Esc`는 답을 미룬다 | engine이 제약 확인을 알릴 때, 다시 열었을 때 열린 확인이 있을 때 |
| 허가 요청 창 | 작업 이름표와 provider가 붙은 제목, 요청 내용, 이유, 선택지 세 개, 허가를 기다리는 다른 작업 수 | 허가 요청 도착 |
| 입력 요청 창 | 허가 요청 창과 같은 제목, 요청 설명, 칸 목록(포커스한 칸만 펼침), 필수 표시 `*`, 오류 한 줄, 키 안내, 답을 기다리는 다른 요청 수. URL 요청은 설명과 링크 | 입력 요청 도착 |
| 작업 목록 화면 | 필터 전체, 확인 필요, 실행 중, 대기, 보류, 끝남, 묶음 채팅, 폴더, 상태, 작업과 그 아래 subagent와 자식 채팅, 작업 상세. 키 `r`과 `g`는 한 줄 입력으로 채팅 이름과 묶음을 바꾸고 `RenameChat`, `SetChatGroup`으로 engine에 저장한다. 앞뒤 공백은 지우고 비우면 이름과 묶음을 지운다. 기본 범위는 현재 채팅의 기본 폴더에서 만든 채팅이고 키 `a`로 모든 폴더로 넓히고 되돌린다. 필터 줄 끝에 범위(`현재 폴더`, `모든 폴더`)를 보인다. 채팅이나 현재 폴더를 알 수 없으면 범위로 거르지 않는다(초안) | `/tasks` 실행, `engine` 상태 변경 때 선택 유지 |
| 제약 목록 화면 | 유효 제약 줄(`번호 · 범위 · 규칙 한 줄`), 확인 필요 줄, 마지막 전환에 들어갔는지 표시(`전환 포함`, `생략`), `Tab`으로 바꾸는 변경 내역(시각, 종류, 주체, 규칙). 키 `d` 해제, `x` 잘못 등록, `u` 되돌리기, `Esc` 닫기 | `/constraints` 실행, 제약 변경 알림 때 선택 유지 |
| 전체 기록 | 도구 셀 전체와 줄인 셀을 펼친 대화 기록 | `Ctrl+T` 입력 |
| 사용량 화면 | 새 입력, 캐시 읽기, 캐시 쓰기, 출력, 추론, router 호출과 예상 비용, 맥락 정리, 채점, 여러 턴 합계 행 끝의 `n 토큰 · n 턴`. provider·모델마다 한 행, router 한 행 | `/usage` 실행, 키 `d`, `w`로 범위 변경 |
| router 버전 화면 | router 버전 목록, 버전별 router와 보정값과 ECE, 질문별 목표 틀림 비율과 기준값과 최근 200건 틀림과 판단 수 | `/router use` 실행 |
| 모델 선택 창 | 고정할 수 있는 모델 목록(`provider · 모델 이름` 줄), 지금 고정한 모델 표시, 키 안내. 목록이 오기 전에는 불러오는 중, 비었으면 안내 한 줄 | `/model` 실행(`/model codex`처럼 provider를 주면 그 provider 모델만), 목록 알림 도착 |
| 채팅 선택 창 | 이어 열 채팅 목록(`번호. #채팅 id · 경과 · [폴더 ·] [이름 ·] 첫 입력` 줄), 고른 줄 강조, 키 안내. 목록이 화면보다 길면 고른 줄이 보이게 스크롤. 대화 화면을 열기 전에 `saturn --resume`이 연다([engine 수명과 복구](engine-lifecycle.md#채팅-폴더와-이어-열기)) | `saturn --resume`, `saturn --resume all` 실행 |
| 학습 확인 창 | 채점 후보 수, 채점 모델, 예상 토큰, 기준값 조정 대상, 모델 추가 학습 여부 | 실행 조건을 채운 `/train` 실행 |

router 키 입력 창에서 받는 키의 처리는 [router 키 보호](router-key-security.md)에, 폴더 설정 신뢰 규칙은 [설정](settings.md)에 있다.

종료 확인 창은 닫으려 할 때 `engine`에 `PrepareExit`을 보내고 `ExitPlan`을 받은 뒤에 뜬다. 닫는 키는 입력창의 `Ctrl+D`와 유휴일 때의 `Ctrl+C`, 창의 종료 선택지다. `Ask`일 때만 창이 뜨고, `Close`는 그대로 닫고, `Notice`는 닫은 뒤 터미널에 한 줄을 남긴다. 창은 다른 창 위에 덮고 `Esc` 뒤에 아래 창이 그대로 남는다. 첫 선택은 작업을 잃지 않는 `계속 실행`이다. `engine`의 답을 기다리는 동안 닫기를 한 번 더 누르면 기다리지 않고 닫는다. router 키 입력 창에서는 `engine`이 요청을 받지 않으므로 묻지 않고 닫는다. 값별 규칙은 [engine 수명과 복구](engine-lifecycle.md#tui-종료-뒤-동작)에 있다.

멈춤 확인 창은 하던 작업과 반대되는 입력(충돌 입력)을 provider가 끼워 넣기로 받지 않았을 때 뜬다. engine이 입력을 `ConfirmStop` 사유의 대기로 알리면 창을 띄우고, 입력이 다른 상태가 되면(다른 TUI가 먼저 답했거나 현재 작업이 끝나 다음 차례로 갔을 때) 창을 지운다. 첫 선택은 작업을 멈추지 않는 `대기`이고, `Esc`와 `Ctrl+C`도 `대기`다. 선택은 `AnswerStopConfirm`으로 보내고, 멈추는 일과 입력 실행은 engine이 한다([충돌 입력](input-handling.md#충돌-입력)). 질문과 선택지는 영어 문구가 있고, 질문은 `~할까요?` 형식을 따른다.

사용량 화면은 provider·모델마다 한 행, router마다 한 행을 보인다. 행 이름은 `codex · gpt-5.6-terra`, `claude · opus`, `router · jev` 형식이고, 모델을 보고하지 않았으면 provider 이름만 쓴다. `/usage`는 인자 없이 지금 채팅의 사용량을 열고, 화면에서 `d`는 모든 채팅의 최근 24시간, `w`는 모든 채팅의 최근 7일로 바꾼다. 같은 키를 다시 누르면 지금 채팅으로 돌아가고, 범위 이름과 키 안내는 화면에 보인다. 범위는 지금부터 거슬러 센 시간이고 달력 날짜가 아니다. 각 행은 고른 범위의 합계다. `saturn usage`는 지금 폴더에서 가장 최근에 입력을 접수한 채팅의 사용량을 표로 쓰고, `--day`는 최근 24시간, `--week`는 최근 7일(모든 채팅)로 바꾼다. 두 옵션은 함께 쓸 수 없고, 그 폴더에 채팅이 없으면 오류로 끝낸다. Claude Code의 `/usage`와 `/stats`가 기간을 키로 바꿔 보는 방식을 따랐다. provider 행의 토큰은 턴 값의 합이고, session 누적 보고는 같은 계열의 직전 누적을 뺀 값을 더한다. 여러 턴을 합친 행은 끝에 `n 토큰 · n 턴`을 보이고, 턴 수는 그 행에 합친 실행 수다. 중간 보고가 빠진 누적 보고는 그사이 보고 없는 실행까지 턴으로 센다. router 행은 호출 수와 입력·출력 토큰을 보인다. 예상 비용, 맥락 정리, 채점처럼 기록 저장소에 없는 값은 `-`로 표시한다. 지어낸 값을 보이지 않기 위해서다.

### 작업 목록 조회

`/tasks`는 `ListTasks`를 보내고 `engine`은 `TaskList`로 답한다. 항목은 작업 글자(이름표)를 가진 메인 작업 하나이고, 채팅 번호와 작업 번호 순서로 보낸다. 항목마다 채팅 이름, 묶음, 채팅의 기본 폴더, 상태, 허가 대기 여부, 지금 도는 하위 에이전트 수를 싣는다. 이름이 없는 채팅은 `#채팅 번호`로 보인다. 상태는 결과 불명(`NeedsCheck`), 보류, 허가 기다림, 입력 기다림, 답은 나왔지만 하위 에이전트가 남음, 실행 중 순서로 정한다. 시작을 기다리는 작업, 턴을 마치고 쉬는 작업, 끝나 이름표를 돌려준 작업, 닫은 보류는 목록에 없다. 채팅 이름과 묶음은 `RenameChat`, `SetChatGroup`으로 바꾸고 바꾼 TUI가 `ListTasks`로 다시 받는다. 다른 TUI에는 알리지 않는다(초안). 목록에 작업이 없는 채팅은 보이지 않는다. 다른 Saturn 프로세스가 실행 중인 채팅(`busy_elsewhere`)은 `engine`이 사용자당 하나라 지금은 늘 거짓이고, 자식 채팅은 아직 없어 하위 에이전트만 센다.

### 이름표

작업 이름표 `[A]`는 살아 있는 작업이 둘 이상이거나 대기 줄이나 보류 줄이 있을 때만 보인다. 작업이 하나이고 대기와 보류가 없으면 이름표를 숨긴다. 이름표가 가리킬 대상이 하나뿐일 때 줄을 짧게 두기 위해서다. 끝난 작업의 글자는 비어 있는 글자 중 가장 앞 글자로 다시 쓴다. 끼워 넣은 입력의 에코에는 그 입력이 합쳐진 작업의 이름표를 붙인다. provider가 띄운 subagent는 이름표 없이 부모 작업 줄 아래에 흐리게 접어 보인다.

### 상태판 줄 순서

상태판은 실행 줄, 판단 줄, 학습 줄, 대기 줄, 보류 줄, 알림 줄 순서로 줄을 쌓고, 같은 종류 안에서는 접수 순서를 따른다. 줄이 생기거나 사라져도 다른 줄끼리의 상대 위치는 유지한다. 사용자가 보던 줄이 갑자기 다른 자리로 뛰는 일을 막기 위해서다. 보류 줄을 뺀 나머지 줄은 그 항목이 끝나면 지운다.

판단 줄은 판단 방식과 관계없이 같은 문구를 쓰고 근거와 확률은 보이지 않는다. 입력이 대기, 끼워 넣기, 새 작업 중 어디로 가는지의 규칙은 [입력 처리](input-handling.md)에 있다.

### 피드백 질문

피드백 질문은 입력 에코 다음 줄에 뜬다. 어떤 판단에서 묻는지는 [router 학습](router-training.md)의 확률 q 규칙을 따른다. 질문은 8초 안에 답이 없으면 사라진다. 사용자가 `2`(틀림)로 답했고 그 입력이 아직 보내지지 않았으면 바로 새 작업으로 실행할지 묻는 바로잡기 제안을 보인다. TUI가 붙어 있지 않은 동안에는 피드백 질문을 건너뛴다([engine 수명과 복구](engine-lifecycle.md)).

### 허가 요청 창

허가 요청은 도착한 순서대로 한 번에 하나씩 창으로 뜬다. 창은 뜬 뒤 1초 동안 키 입력을 받지 않는다. 여러 TUI가 붙어 있을 때 다른 클라이언트가 먼저 답하면 창을 지운다. 선택지는 `이번만 허용`(`y`), `항상 허용`(`a`), `거부`(`d`, `Esc`) 셋이다. Claude와 Codex CLI의 허가 창과 같은 모양이다. `항상 허용`은 허용 규칙으로 Saturn 기록 저장소에 저장되고 provider 설정 파일은 바뀌지 않는다([권한](permissions.md)). `거부`는 다르게 하라는 말을 함께 남길 수 있고, 그 입력 방식은 정해지지 않았다. 도구 호출이 시작되고 3초 안에 허가 요청이나 진행 이벤트가 없으면 실행 줄에 `도구 사용 허가 준비 중 · {provider}`(문구 초안)를 보이고 계속 기다린다. 허가 요청이 오면 그 표시를 지우고 창을 띄운다.

### 입력 요청 창

provider의 입력 요청([입력 요청](input-requests.md))은 도착한 순서대로 한 번에 하나씩 창으로 뜬다. 허가 요청 창이 함께 있으면 허가 요청 창이 먼저다. 창은 뜬 뒤 1초 동안 키 입력을 받지 않고, 다른 클라이언트가 먼저 답하면 지운다. 칸은 종류별로 입력한다. 글과 숫자와 정수는 글자를 쓰고, 예와 아니오는 `Space`나 `y`, `n`으로 정하고, 단일 선택과 다중 선택은 `↑`, `↓`로 줄을 옮겨 `Space`나 `Enter`로 고른다. 목록에 없는 글을 쓸 수 있는 선택 칸은 마지막 줄 `직접 입력`에 글을 쓴다. 필수 칸이 비었거나 숫자를 읽을 수 없으면 그 칸으로 돌아가 이유를 보이고 보내지 않는다. 비밀 칸은 입력을 `•`로 가린다. `거절`(`Ctrl+D`)은 묻는 내용에 답하지 않겠다는 뜻이고 `취소`(`Esc`)는 요청을 없던 일로 한다. URL 요청은 설명과 링크만 보이고 Saturn은 링크를 열지 않는다. 사용자가 직접 열고 `Enter`로 계속한다. 답을 기다리는 동안 실행 줄은 `입력 기다림`이다.

### 채팅 이동

작업 목록에서 다른 채팅을 고르거나 `n`으로 새 채팅을 만들면 TUI는 연결을 유지한 채 붙은 채팅만 바꾼다. 순서는 다음과 같다.

1. TUI가 대화 기록, 실행 영역, 허가 요청과 입력 요청 대기열, 상태판을 비우고 같은 연결로 `Attach`(옮길 채팅 `chat`, 새 채팅이면 비움)를 보낸다.
2. `engine`은 붙은 채팅을 옮길 채팅으로 바꾸고 `StartInfo`, `HistoryChunk`, 답을 기다리는 허가 요청 순서로 보낸 뒤 `Attach`에 응답한다([engine 수명과 복구](engine-lifecycle.md)).
3. 떠난 채팅의 작업은 그대로 계속된다.

`Detach`는 보내지 않는다. `Detach`는 연결을 끊고 마지막 TUI 이탈로 세어 `on_exit`를 적용하므로, 채팅을 옮기려고 보내면 연결이 끊기고 `on_exit`가 `stop`일 때 떠난 채팅의 작업이 멈춘다. 입력창, 입력 기록, provider 명령 목록은 채팅을 옮겨도 유지한다.

### 화면 언어와 출력 방식

화면 문구는 운영체제 언어에 따라 영어와 한국어 중 하나로 고른다. engine은 화면 문구를 만들지 않고 알림 종류와 값만 보내며, 문구는 TUI의 번역표가 고른다. `saturn` 명령의 출력, 오류, 도움말도 같은 언어 판정과 같은 번역표를 쓴다. 도움말은 실행할 때 언어에 맞는 문구를 넣는다. provider와 검사기가 낸 원문(모델 답, 오류 원문)과 로그는 번역하지 않고, clap이 만드는 `Usage:` 같은 고정 문구도 영어로 남는다. 대기 줄과 보류 줄의 버튼은 전체 화면 방식에서 클릭할 수 있다. 파이프와 CI처럼 화면이 없는 환경에서는 전체 화면 대신 plain 출력을 쓰고, 같은 명령은 두 방식에서 같은 결과를 낸다. plain을 켜는 조건은 정해지지 않았고, 지금 구현은 표준 입력이나 표준 출력이 터미널이 아니면 plain으로 시작한다(초안, [#57](https://github.com/woonyong-choi/saturn/issues/57)).

색은 오류와 중단 표시(`중단됨`, `거절됨`, 작업 실패의 원인)에만 빨간색을 쓰고 나머지는 색 없이 글자 속성만 쓴다. 문구 형식은 Claude Code에 맞춘다. 서술문의 해요체(`넣었어요`)와 끝 마침표는 쓰지 않고, 질문은 물음표로 끝낸다. 영어는 문장 첫 글자만 대문자로 쓰고 같은 뜻은 같은 낱말로 쓴다. 키 안내 줄(`Enter 확인 · Esc 닫기`)과 버튼(`[보내기]`)은 이 표의 대상이 아니다.

| 종류 | 형식 | 예 |
|---|---|---|
| 상태 표시(상태판, 라벨, 작업 상태) | 명사형, 마침표 없음 | `작업 중` / `Working`, `입력 기다림` / `Waiting for input`, `중단됨` / `Interrupted` |
| 대화 기록 알림 | `상태 · 안내`, 합니다체, 마침표 없음 | `권한 설정 변경됨 · 다음 요청부터 적용됩니다` / `Permission settings changed · Applies from your next request` |
| 오류 | `무엇을 하지 못했습니다: 대상`, 합니다체 | `engine을 시작하지 못했습니다: {binary}` / `Failed to start engine: {binary}` |
| 안내와 질문 | 안내는 `~하세요`, 질문은 `~나요?`/`~할까요?` | `숫자를 입력하세요`, `판단이 맞았나요?` |

### 키

| 키 | 동작 | 적용 영역 |
|---|---|---|
| `Ctrl+T` | 전체 기록 표시 | 모든 영역 |
| `Ctrl+Z` | 화면 일시 중지, `fg` 뒤 복원 | 모든 영역 |
| `0` | 피드백 질문 해제 | 대화 기록 |
| `1` | 피드백 질문에 맞음 답, `/feedback 1`과 동일 | 대화 기록 |
| `2` | 피드백 질문에 틀림 답, `/feedback 2`와 동일 | 대화 기록 |
| `Enter` | 보류 닫기 확인에서 보류 종료 | 상태판 |
| `Esc` | 보류 닫기 확인에서 보류 유지 | 상태판 |
| `Enter` | 명령 목록에서 고른 명령의 전체 경로를 입력창에 기입, 값 목록에서 값 선택 | 팝업 |
| `Esc` | 팝업 해제, 입력 토큰이 바뀔 때까지 재표시 없음 | 팝업 |
| `Tab` | 명령의 전체 경로까지 완성 | 팝업 |
| `↑`, `↓` | 목록 이동 | 팝업 |
| `!` | 줄 앞에서 셸 명령 바로 실행, 결과를 화면에 표시하고 메인 에이전트의 다음 입력에 첨부 | 입력창 |
| `$` | 단어 맨 앞에서 메인 에이전트 provider의 스킬 목록 표시, 다른 provider 스킬은 `$공급자 이름` | 입력창 |
| `/` | 명령 목록 표시 | 입력창 |
| `?` | 빈 입력창에서 단축키 안내 표시 | 입력창 |
| `@` | 파일 목록 표시 | 입력창 |
| `Alt+Enter`, `Ctrl+J`, `Shift+Enter` | 줄바꿈 | 입력창 |
| `Alt+↑`, `Shift+←` | 대기나 판단 중인 가장 최근 입력을 취소하고 원문을 입력창으로 이동 | 입력창 |
| `Ctrl+C` | 뷰 해제, 검색 취소, 초안 삭제 순서로 하나 처리, 그 밖에는 실행 중이면 모든 작업 중지와 보류, 유휴이면 종료 | 입력창 |
| `Ctrl+D` | 빈 입력창에서 종료 | 입력창 |
| `Ctrl+G` | 외부 에디터로 초안 편집 | 입력창 |
| `Ctrl+K` | 초안 글자 잘라 보관 | 입력창 |
| `Ctrl+R` | 입력 기록 검색 | 입력창 |
| `Ctrl+Y` | 잘라 둔 글자 복원 | 입력창 |
| `Enter` | 입력 제출, 유휴이면 새 작업, 실행 중이면 router가 끼워 넣기, 새 작업, 대기 중 하나 선택 | 입력창 |
| `Esc` | 팝업과 선택 해제, 작업 중지 없음 | 입력창 |
| `Tab` | 유휴이면 `Enter`와 동일, 실행 중이면 관계 판단 없이 대기, 보낼 때 router 1회 | 입력창 |
| `↑`, `↓` | 입력창이 비었거나 불러온 기록 그대로일 때 입력 기록 이동 | 입력창 |
| `Enter` | 키 확인 | router 키 입력 창 |
| `Esc` | 종료 | router 키 입력 창 |
| `1`, `y` | 적용하고 계속 강조 | 폴더 설정 신뢰 창 |
| `3`, `q`, `Esc`, `Ctrl+C` | 종료 | 폴더 설정 신뢰 창 |
| `Enter` | 강조한 선택지 확정 | 폴더 설정 신뢰 창 |
| `↑`, `↓` | 선택지 이동 | 폴더 설정 신뢰 창 |
| `↑`, `↓` | 선택지 이동 | 종료 확인 창 |
| `Enter` | 고른 선택지 확정 | 종료 확인 창 |
| `Esc`, `Ctrl+C` | 닫기 취소, 작업 중지 없음 | 종료 확인 창 |
| `↑`, `↓` | 선택지 이동 | 멈춤 확인 창 |
| `Enter` | 고른 선택지 확정 | 멈춤 확인 창 |
| `Esc`, `Ctrl+C` | `대기` 선택, 작업 중지 없음 | 멈춤 확인 창 |
| `Enter` | 선택 | 보류 재개 질문 |
| `↑`, `↓` | 선택지 이동 | 보류 재개 질문 |
| `y` | 이번만 허용 | 허가 요청 창 |
| `a` | 항상 허용, Saturn 기록 저장소에 저장 | 허가 요청 창 |
| `d`, `Esc` | 거부하고 계속 | 허가 요청 창 |
| `Tab`, `Shift+Tab` | 다음 칸, 앞 칸 | 입력 요청 창 |
| `↑`, `↓` | 선택 칸의 줄 이동, 칸의 끝에서는 이웃 칸으로 이동 | 입력 요청 창 |
| `Space` | 예와 아니오 바꾸기, 선택 줄 고르기와 해제 | 입력 요청 창 |
| `Enter` | 단일 선택 고르기, 다음 칸으로 이동, 마지막 칸이면 보내기. URL 요청은 계속 | 입력 요청 창 |
| `Ctrl+D` | 거절. URL 요청은 `d`도 같다 | 입력 요청 창 |
| `Esc` | 취소 | 입력 요청 창 |
| `?` | 도움말 | 작업 목록 화면 |
| `Enter` | 그 채팅으로 이동해 해당 작업 결과로 스크롤 | 작업 목록 화면 |
| `Esc` | 화면 종료 | 작업 목록 화면 |
| `Tab`, `Shift+Tab` | 필터 변경 | 작업 목록 화면 |
| `a` | 폴더 범위 바꾸기(현재 폴더, 모든 폴더) | 작업 목록 화면 |
| `c` | 보류 작업 재개 | 작업 목록 화면 |
| `d` | 대기 취소, 확인 한 줄 뒤 보류 종료 | 작업 목록 화면 |
| `f` | 검색 | 작업 목록 화면 |
| `g` | 묶음 변경 | 작업 목록 화면 |
| `n` | 새 채팅 | 작업 목록 화면 |
| `r` | 채팅 이름 변경 | 작업 목록 화면 |
| `s` | 대기 입력 전송 | 작업 목록 화면 |
| `↑`, `↓` | 목록 이동 | 작업 목록 화면 |
| `Enter` | router 실제 모델 상세 표시 | 사용량 화면 |
| `d` | 최근 24시간 사용량으로 바꾸기, 이미 보고 있으면 지금 채팅으로 돌아가기 | 사용량 화면 |
| `w` | 최근 7일 사용량으로 바꾸기, 이미 보고 있으면 지금 채팅으로 돌아가기 | 사용량 화면 |
| `Esc` | 화면 종료 | 사용량 화면 |
| `Enter` | 버전 상세 표시 | router 버전 화면 |
| `Esc` | 화면 종료 | router 버전 화면 |
| `r` | 1차 영점으로 복귀, `/train --reset-thresholds`와 동일 | router 버전 화면 |
| `t` | 고른 버전에서 다시 학습, `/train --from`과 동일 | router 버전 화면 |
| `u` | 확인 한 줄 뒤 고른 버전 사용, `saturn router use`와 동일(명령줄은 `--yes`로 확인을 건너뜀) | router 버전 화면 |
| `↑`, `↓` | 목록 이동 | 채팅 선택 창 |
| `Enter` | 고른 채팅 열기 | 채팅 선택 창 |
| `Esc`, `Ctrl+C` | 채팅을 열지 않고 취소 | 채팅 선택 창 |
| `Enter` | 선택 | 학습 확인 창 |
| `↑`, `↓` | 모델 이동 | 모델 선택 창 |
| `Enter` | 고른 모델을 이 채팅의 고정 모델로 정하고 창 닫기 | 모델 선택 창 |
| `Esc` | 취소, 고정 모델은 그대로 | 모델 선택 창 |
| `Esc` | 취소 | 학습 확인 창 |
| `↑`, `↓` | 선택지 이동 | 학습 확인 창 |

### 상태 표시

| 표시 | 뜻 |
|---|---|
| `[A]` | 작업 이름표 |
| `> [A] 원문` | 판단이 끝난 입력의 에코 |
| `⠙ [A] 작업 중` | 출력이 아직 없는 실행 중 작업 |
| `⠙ [A] claude · opus · 1분 · 하위 에이전트 2개 실행 중` | provider subagent가 도는 작업 |
| `생각 중` | 실행 줄의 하는 일, provider가 생각하는 중 |
| `파일 읽는 중` | 실행 줄의 하는 일, 파일 조회 |
| `파일 수정 중` | 실행 줄의 하는 일, 파일 수정 |
| `명령 실행 중` | 실행 줄의 하는 일, 명령 실행과 명령 앞 40칸 |
| `허가 기다림` | 실행 줄의 하는 일, 허가 응답 대기와 경과 시간 정지 |
| `입력 기다림` | 실행 줄의 하는 일, provider 입력 요청 응답 대기와 경과 시간 정지 |
| `도구 사용 허가 준비 중 · codex` | 실행 줄의 하는 일, 도구 호출 시작 뒤 3초 안에 허가 요청이나 진행 이벤트가 없을 때(문구 초안) |
| `응답 없음 5분` | 실행 줄의 하는 일, 마지막 provider 이벤트 뒤 5분(초안) 이상 이벤트가 없을 때 분 단위로 갱신. 허가나 입력을 기다리는 동안은 보이지 않고 이벤트가 오면 지움. 자동으로 멈추지 않고 `Ctrl+C`로 멈춘다. 영어는 `No response 5m`([provider 연결과 session](providers-and-sessions.md#무응답-표시)) |
| `맥락 정리 중` | 실행 줄의 하는 일, 맥락 정리 |
| `공급자 전환 중` | 실행 줄의 하는 일, provider 전환 |
| `Token -` | 사용량 보고 전 |
| `⠹ [D] 판단 중` | router가 입력을 판단하는 중 |
| `⠼ [학습]` | `/train` 진행, 단계와 채점 건수와 경과와 토큰 |
| `· [C] 대기 · A 다음` | 실행 중인 작업 A 뒤에 보낼 입력 |
| `· [C] 대기 · 판단 차례` | 같은 채팅의 앞 입력 판단을 기다리는 입력, `[보내기]` 없이 `[취소]`만 표시 |
| `· [C] 대기 · 라우터 연결 기다림` | router 연결을 기다리는 입력 |
| `· [C] 대기 · 쓰기 차례` | 다른 에이전트의 쓰기나 읽기 전용으로 접수한 실행이 끝나기를 기다리는 입력 |
| `· [C] 대기 · 맥락 정리 뒤` | session 교체가 끝나기를 기다리는 입력 |
| `· [C] 대기 · 모든 작업 뒤` | 모든 작업이 끝난 뒤 실행할 session 변경 명령 |
| `· [C] 대기 · 멈출지 확인 중` | 끼워 넣기를 받지 않은 충돌 입력이 멈춤 확인 창의 답을 기다림. 영어는 `Confirming stop` |
| `[보내기] [취소]` | 대기 줄 버튼, `/send`, `/cancel`과 같은 동작 |
| `‖ [A] 보류` | 멈춘 작업이나 보내지 않은 입력, `/continue`로 재개 |
| `[이어서] [취소]` | 보류 줄 버튼, `/continue`, `/cancel`과 같은 동작 |
| `‖ 멈춤 · [A] [C] 보류됨 · /continue 로 이어서` | 중지 결과, 멈춘 작업과 보내지 않은 입력의 보류 |
| `{provider}가 크래시로 끊긴 하위 에이전트를 다시 시작해 작업을 멈췄습니다 · 이어 가려면 /continue` | 크래시로 끊긴 하위 에이전트의 이벤트가 provider에서 다시 와서 채팅의 작업을 멈춤. 영어는 `{provider} restarted a subagent cut off by the crash, so the task was stopped · /continue to resume` |
| `멈춤 확인 안 됨 · N개 남음` | provider 프로세스 묶음 밖에 남은 프로세스 N개 |
| `‖ [E] 보류를 닫을까요?` | 보류 닫기 확인, 보내지 않은 입력 취소, 수정된 파일 유지 |
| `[E] 보류를 닫았습니다` | 보류 종료 완료 |
| `자동 판단 일시 중단` | router 호출 일시 실패, 질문별 대체 규칙 적용 |
| `판단 모델 연결 끊김` | router 호출 연속 3회 실패, 입력 접수는 계속하고 현재 모델로 처리 |
| `바로 반영 준비 중 (codex)` | 해당 provider의 끼워 넣기 확인 전, 끼워 넣기 대신 대기 처리 |
| `라우터 연결 없음 · 차례에 보냅니다` | 바로 보내기가 router를 부르지 않아 engine이 더는 보내지 않는 안내. 화면 쪽 표시는 남아 있고 제거 여부는 미정 |
| `전달 중` | 에이전트로 입력 전달 중, 취소 불가 |
| `반영됨` | 에이전트에 입력 전달 완료, 취소 불가 |
| `거절됨` | 보내기 전 확정 실패가 3번 이어져 입력을 보내지 못함, 입력 에코 끝에 빨간색으로 붙고 작업 실패의 원인은 빨간색으로 다음 줄에 보임. 영어는 `Rejected` |
| `중단됨` | 중단이나 크래시로 결과를 모르는 도구 실행, 도구 셀 아래 빨간색 한 줄. 작업이 실패하거나 멈추거나 결과 확인 필요가 될 때 결과 없이 남은 호출에 붙고, 정상으로 끝난 작업에는 붙이지 않음. 영어는 `Interrupted`. Claude Code는 중단된 도구 결과를 오류로, Codex는 aborted로 보임 |
| `[A] codex · 45초 · Token 3,210` | 결과 머리줄, 마지막으로 답한 provider와 경과와 토큰 |
| `[A] codex · 45초 · 실패` | 작업 실패, 다음 줄에 원인 한 줄 |
| `[A] 결과 확인 필요 · /continue A` | 결과 불명, 보류 줄과 함께 표시 |
| `[A] 맥락 정리 후 이어서 진행` | 맥락 정리 뒤 같은 작업 계속 |
| `고정 제약이 길어 맥락 정리를 미룹니다` | 패킷의 고정 구역이 `P_hard`도 넘어 새 session으로 옮기지 못함, 다음 줄부터 제약 목록 |
| `제약 등록됨 · {규칙}` | 제약 등록(`Added`). 영어는 `Constraint added · {rule}`. 규칙은 사용자 원문 그대로이고 한 줄에 맞게 줄인다 |
| `제약 합침 · {기존 규칙}` | 같은 뜻의 제약이라 새로 만들지 않고 합침(`Merged`). 영어는 `Constraint merged · {rule}` |
| `제약 대체됨 · {앞 규칙} → {새 규칙}` | 새 제약이 앞 제약을 대체(`Superseded`). 영어는 `Constraint replaced · {earlier} → {new}` |
| `제약 해제됨 · {규칙}` | 해제(`Released`), 사용자 해제, 확인 답, 입력 취소를 같은 줄로 보임. 영어는 `Constraint released · {rule}` |
| `제약 되돌림 · {규칙}` | 변경 되돌리기(`Restored`). 영어는 `Constraint restored · {rule}` |
| `제약 {N}개 생략 · /constraints에서 확인하세요` | 전환 패킷의 제약 칸이 가득 차 N개를 넣지 못함. 영어는 `{N} constraints omitted · Check /constraints` |
| `맥락 한도 초과로 멈춤 · /continue로 다시 시도하세요` | 패킷이 맥락 한도로 거절돼 줄여 다시 보냈지만 들어가지 않거나 줄일 수 없어 보내지 않고 멈춤. 영어는 `Stopped over the context limit · Retry with /continue` |
| `[A] codex → claude로 전환` | 작업의 provider 전환 |
| `권한 설정 변경됨 · 다음 요청부터 적용됩니다` | 채팅 중 설정이 바뀌었는데 작업 중이라 다시 시작을 턴 끝으로 미룸. 바로 다시 시작하면 이 줄 없이 아래 줄만 남김. 영어는 `Permission settings changed · Applies from your next request` |
| `Codex 다시 시작함 · 변경된 권한 설정을 적용했습니다` | 바뀐 권한 설정을 적용하려고 provider 연결을 다시 시작함(바로 또는 턴이 끝난 뒤). Claude도 같은 줄. 영어는 `Restarted Codex · Applied the changed permission settings` |
| `이번 요청 · codex Token 4,120 · 라우터 3회 Token 9,870 · 2분 31초` | 모든 작업이 끝난 순간의 합계, provider별 토큰과 router 호출과 경과 |
| `[A]에 이어서 보냄 · 판단이 맞았나요? (선택)  1 맞음  2 틀림  0 닫기` | 피드백 질문 |
| `[B] 바로 새 작업으로 실행할까요? [실행] [그대로]` | 틀림 답 뒤 아직 보내지 않은 입력의 바로잡기 제안 |
| `채점할 판단 83 / 200건 · 200건이 쌓이면 실행할 수 있습니다` | 채점할 판단 부족으로 `/train` 실행 불가 |
| `맥락 38K/200K` | 현재 활성 맥락 크기와 Saturn 기준 |
| `맥락 미확인` | 맥락 크기 측정 불가 |
| `[붙여넣은 내용 1,204자]` | 1,000자를 넘는 붙여넣은 내용 |
| `폴더 설정 오류 · 이전 설정 번호 12로 계속 · 줄 7: ...` | 설정 검사 실패, 이전 설정으로 계속. 앞 이름은 실패한 층(기본, 사용자, 폴더, 채팅, 실행 설정) |
| `폴더 설정의 무시한 항목 · router.endpoint` | 폴더 설정의 사용자 전용 키를 무시함 |
| `!` | 작업 목록의 허가 필요 작업 |
| `?` | 작업 목록의 결과 확인 필요 작업 |
| `다른 Saturn에서 실행 중` | 다른 Saturn 프로세스가 실행 중인 채팅, 작업 목록에서 읽기 전용 |
| `작업 2개 계속 실행 중 · saturn으로 다시 여세요` | TUI를 닫은 뒤 터미널에 남기는 한 줄. `on_exit`가 `background`이거나 종료 확인 창에서 `계속 실행`을 고르고 작업이 남았을 때. 영어는 `Tasks still running: 2 · Reopen with saturn` |
| `기록 저장소 v2로 옮김` | 시작할 때 기록 저장소 스키마를 이관함, 첫 TUI의 상태판 알림 줄 |
| `모델 미보고` | 작업 상세에서 provider의 모델 보고 없음 |

### 초안 값

설계에서 정하지 않아 구현이 정한 값이다. 바꾸면 이 표와 코드를 함께 고친다.

| 대상 | 값 |
|---|---|
| 틱 간격 | 100ms |
| 무응답 표시 기준 | 5분. `NO_RESPONSE_AFTER` 상수 하나, 설정 키 없음 |
| 이름표 글자 | `A`부터 `Z`까지 |
| 작업별 출력 칸 축소 | 화면 높이의 1/4, 최소 1줄, 최대 8줄 |
| 높이가 모자랄 때 | 작업별 출력 칸, 상태판, 팝업 순서로 줄이고 입력창과 바닥줄은 줄이지 않는다 |
| 시작 화면 전환 | 대화 기록에 첫 셀(입력 에코 포함)이 생길 때 |
| 전체 기록 키 | `Esc` 닫기, `↑`, `↓` 스크롤, `Ctrl+T`를 다시 누르면 닫기. 창 화면의 `Ctrl+C`도 뷰 해제 |
| 화면 언어 | `LC_ALL`, `LC_MESSAGES`, `LANG` 중 처음 비어 있지 않은 값이 `ko`로 시작하면 한국어. 영어 문구는 `saturn-terminal/tui/src/i18n/english.rs`의 `ENGLISH`이고, `saturn` 명령도 이 표를 쓴다 |
| 외부 에디터와 셸 | `$VISUAL`, `$EDITOR`, `vi` 순서. 셸은 `$SHELL -c`, 없으면 `sh -c`. 자식 환경에서 `SATURN_KEY` 제거. 에디터 초안 파일은 새 경로에 권한 0600으로 생성 |
| 입력 기록 파일 | 한 줄에 입력 하나, 줄바꿈은 `\n`, 역슬래시는 `\\`, 오래된 것이 위. 쓰기 실패는 경고 로그만 남기고 계속 |
| 결과 머리줄 토큰 | 새 입력, 캐시 쓰기, 출력, 추론의 합. 캐시 읽기는 뺀다 |
| 폴더 설정 신뢰 창 두 번째 선택지 | `적용하지 않고 계속` |
| 끼워 넣기가 아닌 판단의 피드백 질문 머리 | `[B] 새 작업으로 보냄`, `[C] 대기열에 넣음` |
| 셸 명령 셀 | 출력 앞 10줄, 전체 기록에서 전체 |
| `@` 파일 목록 | 2,000개까지. `.git`과 작업 폴더 `.gitignore`의 글로브 없는 이름은 뺀다 |
| 스크롤 | 휠 한 칸 3줄, 맨 위에 닿으면 이전 기록 50단위(입력 또는 실행 하나) 요청 |
| router 키 입력 창 붙여넣기 | 제어 문자를 뺀 글을 가린 입력칸에 넣는다 |
| `/record` | 명령 목록에 넣고 값 목록은 `on`, `off` |
| `/add-dir` | 명령 목록에 넣고 값은 폴더 경로 하나. 명령 이름 뒤 나머지 줄 전체를 경로로 읽어 공백이 들어 있어도 된다. 상대 경로는 TUI의 현재 폴더 기준 절대 경로로, `~/`는 홈 폴더 아래로 바꿔 보낸다(초안). 더한 폴더는 채팅 기록에 저장하고 모든 provider session에 넘긴다. 폴더 설정은 읽지 않는다([engine 수명과 복구](engine-lifecycle.md#채팅-폴더와-이어-열기)). 더한 뒤 안내 한 줄을 대화 기록에 남기고, 열린 session이 있으면 다음 session부터 적용한다고 덧붙인다 |
| `/model` | 명령 목록에 넣고 값 목록은 `codex`, `claude`. 값 없이 실행하면 모든 provider의 모델 창이 열리고, 값을 주면 그 provider 모델만 보인다. 방향키로 고르고 `Enter`로 정한다. 고른 모델은 `SetModel`로 engine에 저장하고, 그 채팅의 모든 입력이 쓴다. 채팅에 붙을 때 engine이 `ModelPinned`로 알려 주므로 TUI를 다시 열거나 채팅을 옮겨도 유지된다. 정한 뒤 안내 한 줄을 대화 기록에 남긴다. provider 고유의 `/model`은 넘기지 않고 Saturn `/model`로 처리한다([모델 고르기](providers-and-sessions.md#모델-고르기)) |
| `/permissions` | 명령 목록에 넣고 값 목록은 `ask`, `edit`, `read-only`, `full`(초안). 값을 주면 채팅 층 모드를 바꾼다. 값 없이 실행하면 현재 모드를 보이는 동작은 아직 없다([#177](https://github.com/woonyong-choi/saturn/issues/177), [권한](permissions.md)) |
| `/constraints` | 명령 목록에 넣고 값은 없다. 제약 목록 화면을 연다. `d`는 `ReleaseConstraint`, `x`는 잘못 등록으로 `ReleaseConstraint`, `u`는 `UndoConstraintChange`를 보내고 확인 창의 답은 `AnswerConstraintAsk`로 보낸다. 요청은 화면이 본 제약 revision을 싣고 낡았으면 engine이 `Stale`로 거절해 목록을 새로 읽는다(초안, [제약](constraints.md#되돌리기)) |

### 요구사항

| 요구사항 | 검증 계획 |
|---|---|
| 작업이 하나이고 대기와 보류가 없으면 이름표를 숨긴다. | `saturn-terminal/tui/src/app/tests.rs`의 `render_single_task_hides_labels`, `render_stacks_transcript_status_composer_and_footer` |
| 패킷이 넘쳐 맥락 정리를 미루면 안내 한 줄과 제약 목록을 보인다. | `saturn-terminal/tui/src/view/transcript.rs`의 `lines_context_deferred_lists_the_constraints` |
| 패킷이 맥락 한도로 거절돼 멈추면 안내 한 줄을 보인다. | `saturn-terminal/tui/src/view/transcript.rs`의 `lines_packet_overflow_tells_the_user_how_to_retry` |
| 상태판 줄이 생기고 사라져도 다른 줄의 상대 위치는 바뀌지 않는다. | `saturn-terminal/tui/src/view/status_board.rs`의 `build_keeps_relative_order_when_line_removed` |
| 닫으려 할 때 `engine`에 닫은 뒤의 처리를 먼저 묻고, 답이 오기 전에는 닫지 않으며 한 번 더 누르면 기다리지 않고 닫는다. | `saturn-terminal/tui/src/app/tests.rs`의 `quit_asks_the_engine_first_and_a_second_quit_closes_without_waiting`, `exit_plan_close_quits_without_a_line`, `exit_plan_nobody_asked_for_is_ignored` |
| `Notice`와 종료 확인 창의 `계속 실행`은 닫은 뒤 터미널에 계속 실행 중인 작업 수와 다시 여는 방법을 한 줄로 남기고, 영어 문구가 있다. | `saturn-terminal/tui/src/app/tests.rs`의 `exit_plan_notice_quits_and_leaves_the_running_count_line`, `exit_plan_notice_line_is_translated`, `exit_confirm_continue_quits_and_leaves_the_running_count_line` |
| 멈춤 확인 창은 `ConfirmStop` 대기 입력이 오면 뜨고 입력이 다른 상태가 되면 지워지며, 첫 선택과 `Esc`는 `대기`이고 `멈추고 실행`은 `AnswerStopConfirm`을 보낸다. 문구에 영어가 있다. | `saturn-terminal/tui/src/app/tests.rs`의 `stop_confirm_window_opens_for_the_asking_input_and_closes_when_it_moves_on`, `stop_confirm_enter_and_escape_answer_wait_and_down_enter_answers_stop`, `saturn-terminal/tui/src/view/stop_confirm.rs`의 `render_shows_the_input_and_both_choices` |
| 제약 확인 창은 확인 알림이 오면 뜨고 대상이 바뀌거나 다른 TUI가 답하면 지워지며, 첫 선택은 지키는 쪽이고 `Esc`는 답을 미룬다. 문구에 영어가 있다. | 확인을 알리고 두 TUI 중 하나가 답한 뒤 창이 지워지는지, `Esc` 뒤 `/constraints`에 확인 필요 줄이 남는지 확인한다. |
| `/constraints`는 유효 제약과 변경 내역을 보이고 `d`, `x`, `u`를 engine 요청으로 보내며 `Stale` 답에 목록을 새로 읽는다. 제약 변경 줄은 `constraint_events`에서 그려 다시 열어도 같다. | 화면을 연 사이 제약을 바꿔 거절과 새로 읽기를, 채팅을 다시 열어 같은 줄을 확인한다. |
| `Ask`는 종료 확인 창을 띄우고, `멈추기`는 `StopAll`을 보낸 뒤 닫고, `Esc`와 `Ctrl+C`는 작업을 멈추지 않고 닫기를 취소한다. | `saturn-terminal/tui/src/app/tests.rs`의 `exit_plan_ask_opens_the_confirm_window_and_waits`, `exit_confirm_stop_stops_every_chat_then_quits_without_a_line`, `exit_confirm_escape_and_ctrl_c_cancel_the_exit_without_stopping_work`, `saturn-terminal/tui/src/view/exit_confirm.rs`의 `render_shows_count_and_both_choices` |
| 작업 목록에서 채팅을 옮기면 `Detach` 없이 `Attach`만 같은 연결로 보내고, `engine`은 연결을 유지한 채 붙은 채팅만 바꾸며 떠난 채팅의 작업을 멈추지 않는다. | `saturn-terminal/tui/src/app/tests.rs`의 `moving_to_another_chat_only_attaches_without_detaching`, `saturn-terminal/engine/src/lifecycle/exit.rs`의 `attach_to_another_chat_on_the_same_connection_is_not_a_detach` |
| 작업 목록 조회는 이름표를 가진 메인 작업을 채팅 이름, 묶음, 폴더, 상태와 함께 채팅과 작업 순서로 보내고, 채팅 이름과 묶음을 바꾸면 다음 조회에 반영하며 이름이 없으면 `#채팅 번호`로 보낸다. | `saturn-terminal/engine/src/lifecycle/tasks.rs`의 `task_list_reflects_the_chat_name_and_group_after_they_change`, `task_list_lists_tasks_of_every_chat_in_chat_order` |
| 작업 목록의 상태는 허가 기다림, 입력 기다림, 보류를 가르고 하위 에이전트 수와 허가 대기를 싣고, 닫은 작업은 뺀다. | `saturn-terminal/engine/src/lifecycle/tasks.rs`의 `task_list_shows_waiting_states_and_running_subagents`, `task_list_shows_held_tasks_and_drops_closed_ones` |
| 화면 문구는 운영체제 언어에 따라 영어와 한국어 중 하나로 고른다. | `saturn-terminal/tui/src/i18n.rs`의 `from_locale_korean_prefix_returns_ko`, `english_covers_every_phrase_constant` |
| 모든 한국어 문구에 영어가 있고, `saturn` 명령의 도움말도 같다. | `saturn-terminal/tui/src/i18n.rs`의 `english_covers_every_phrase_constant`, `saturn-terminal/cli/src/args.rs`의 `help_has_english_for_every_korean_text`, `localized_help_replaces_korean_with_english` |
| 한국어 문구에 해요체 어미와 끝 마침표가 없다. | `saturn-terminal/tui/src/i18n.rs`의 `korean_phrases_follow_claude_code_format` |
| router 연결이 끊겨도 입력창은 입력을 계속 보낸다. | `saturn-terminal/tui/src/app/tests.rs`의 `submit_while_router_disconnected_still_sends_input` |
| `/add-dir`는 폴더 경로를 절대 경로로 바꿔 engine에 보내고, 시작 화면과 안내 줄이 더한 폴더를 보인다. | `saturn-terminal/tui/src/app/tests.rs`의 `add_dir_command_sends_an_absolute_path_relative_to_the_tui_folder`, `add_dir_notice_adds_a_cell_and_updates_the_start_screen_folders`, `saturn-terminal/tui/src/view/start_screen.rs`의 `lines_show_the_chat_folder_and_the_added_folders`, `saturn-terminal/tui/src/view/transcript.rs`의 `lines_folder_added_mentions_the_next_session_only_when_one_is_open` |
| `/model`은 목록 창을 열고, `Enter`는 고른 모델을 engine에 저장하라고 보내고, `Esc`는 아무것도 보내지 않는다. 고정은 채팅에 붙을 때 알려져 창에 표시된다. `/model <provider>`는 그 provider 모델만 요청하고, provider의 `/model`은 넘기지 않는다. | `saturn-terminal/tui/src/app/tests.rs`의 `model_command_asks_for_the_list_and_opens_the_window`, `model_command_with_a_provider_asks_only_for_that_provider`, `model_window_enter_asks_the_engine_to_pin_the_model`, `model_window_escape_sends_nothing`, `pinned_model_notice_marks_the_model_in_the_next_window`, `model_command_is_not_passed_to_the_provider`, `saturn-terminal/tui/src/view/model_picker.rs`의 `selection_stays_inside_the_list`, `list_starts_on_the_pinned_model`, `saturn-terminal/tui/src/commands.rs`의 `parse_model_reads_an_optional_provider_and_keeps_the_provider_command_out` |
| 작업 목록은 기본으로 현재 채팅 폴더의 채팅만 보이고 키 `a`로 모든 폴더를 본다. | `saturn-terminal/tui/src/view/task_list.rs`의 `task_list_defaults_to_the_current_folder_and_the_key_widens_it`, `task_list_does_not_hide_chats_whose_folder_is_unknown`, `saturn-terminal/tui/src/app/tests.rs`의 `task_list_opened_from_a_chat_starts_in_the_chat_folder_scope`, `task_list_key_a_widens_the_scope_to_all_folders` |
| 화면이 없는 파이프와 CI에서도 같은 명령이 같은 결과를 낸다. | `saturn-terminal/tui/src/plain.rs`의 `plain_and_full_screen_cells_use_same_text`, `apply_writes_echo_output_result_and_summary` |

## 미해결 질문

- 메인 에이전트가 아닌 provider의 명령을 고르면 그 provider session을 새로 열지, 메인 전환을 물을지, 거절할지 ([#41](https://github.com/woonyong-choi/saturn/issues/41))
- 실행 줄의 칸 순서를 provider와 모델 먼저로 둘지, 하는 일 먼저로 둘지, 모델 이름을 보고된 그대로 쓸지 별칭으로 쓸지 ([#50](https://github.com/woonyong-choi/saturn/issues/50))
- 상태판 최대 높이를 화면 높이 비율로 둘지, 고정 줄 수로 둘지, 상한을 두지 않을지 ([#51](https://github.com/woonyong-choi/saturn/issues/51))
- 상태판 버튼을 `Shift+Tab` 진입과 방향키로 고를지, `/send`, `/cancel`, `/continue` 명령만 쓸지, 줄마다 번호 키를 줄지 ([#52](https://github.com/woonyong-choi/saturn/issues/52))
- 짧게 끝나는 판단의 판단 줄을 생략할지, 항상 그릴지, 지연 표시와 최소 표시 시간을 둘지 ([#53](https://github.com/woonyong-choi/saturn/issues/53))
- 피드백 질문의 숫자 키를 입력창이 비었을 때만 답으로 받을지, 항상 받을지, `/feedback` 명령으로만 받을지 ([#54](https://github.com/woonyong-choi/saturn/issues/54))
- `/stop`이 진행 중인 `/train`도 멈출지, 학습 전용 중지 명령을 둘지, 학습 줄에 중지 버튼을 둘지 ([#55](https://github.com/woonyong-choi/saturn/issues/55))
- 허가 거절 뒤 다르게 하라는 입력을 접두 초안으로 받을지, 창 안 입력칸으로 받을지, 일반 입력처럼 router에 맡길지 ([#56](https://github.com/woonyong-choi/saturn/issues/56))
- plain 출력을 켜는 조건과 우선순위, 설정 키 이름을 무엇으로 할지 ([#57](https://github.com/woonyong-choi/saturn/issues/57))
- 좁은 가로 폭에서 폭 구간별로 버튼과 칸을 줄일지, 줄 끝부터 말줄임할지, 버튼 대신 명령 안내를 보일지 ([#58](https://github.com/woonyong-choi/saturn/issues/58))
- 빈 입력창에서 `←`로 작업 목록 화면을 열지, `/tasks`로만 열지, 다른 전용 키를 둘지 ([#59](https://github.com/woonyong-choi/saturn/issues/59))
- 바로잡기 제안의 `[실행]`, `[그대로]`를 클릭으로 고를지, 숫자 키로 고를지, 명령으로만 고를지 ([#109](https://github.com/woonyong-choi/saturn/issues/109))
- 위로 스크롤할 때 이전 기록 요청의 기준 위치를 `HistoryChunk`에 실을지, 항목마다 붙일지, engine이 기억할지 ([#110](https://github.com/woonyong-choi/saturn/issues/110))
