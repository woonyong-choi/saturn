# 데이터

## 출처와 보존 범위

현재 연구의 원자료는 저장소 루트 기준 `.local/experiments/real-context-replay/`에 있다. 개인 대화와 모델 원응답을 포함하므로 Git 공개 대상에서 제외한다. 기존 원본 대화는 선택한 입력·답과 출처 위치·원본 SHA를 남기고, 비밀 문자열과 개인 경로를 가린 재생본을 만들었다. SQLite 원본은 읽기 전용으로 접근했다.

| 경로 | 내용 |
|---|---|
| `learning-source.json`, `audit-source.json` | 첫 수집의 선택 대화와 원본 파일·행 위치 |
| `cases.json`, `seal.json` | 첫 재생 입력과 호출 전 봉인 |
| `packets/`, `raw/`, `first-summary.json` | 변환·조건 결함이 있던 첫 수집의 패킷, 원응답과 집계 |
| `corrected/*-source.json`, `corrected/cases.json` | 원래 시각과 완료 표시를 보존한 재생 입력 |
| `corrected/packets/`, `corrected/packets-verified/` | 현재 engine이 생성한 패킷과 재생성한 동일 bytes |
| `corrected/calls-plan.json`, `corrected/collection-seal.json` | 실제 전송할 맥락·질문·순서와 호출 전 파일 hash |
| `corrected/raw/*/context.txt`, `question.txt` | 각 호출의 전송 입력 |
| `corrected/raw/*/ready.jsonl`, `answer.jsonl`, `tail.jsonl` | Claude의 인수·답·종료 전 나머지 수신 줄 |
| `corrected/raw/*/stdout.jsonl`, `combined-input.txt` | Codex의 원출력과 실제 결합 입력 |
| `corrected/raw/*/stderr.txt`, `request.json`, `result.json` | 오류 출력·명령·시각·사용량·파싱 결과 |
| `code-at-collection/`, `corrected/code-at-collection/` | 호출 당시 수집 코드. 이후 포맷·재현 진입점 변경과 구분 |
| `.local/verification/cleanup-context/` | 예비 합성 시험, 수정 전 실패, 전체 검사·부하 검사·CLI 도움말·기존 변경 상태 |

`raw`에는 응답 누락·오답·인수 실패도 남긴다. 사후 채점과 표현 교정은 `results/summary.json`에만 반영하며 모델 원응답은 수정하지 않는다. 첫 합성 탐색의 최초 누락 덤프는 남아 있지 않다. 이 불완전한 탐색 자료를 검증된 표본으로 합치지 않는다.

## 파일과 해시

[SHA256SUMS](SHA256SUMS)는 저장소 루트 기준 경로별 SHA-256이다. 연구 보고서, 원응답, 전송 입력과 실행 코드를 연결한다. `run.sh verify`는 그 bytes와 수집·채점 분모를 확인한다. 원자료가 없는 복제본에서는 검증·재분석할 수 없다. 로컬 보관 묶음의 위치와 hash는 [archive.json](archive.json)에 기록한다.

`results/summary.json`의 `answer_correct`는 답만의 정답 수이고 `correct`는 인수 규칙까지 포함한 사전 기준이다. `failure_kinds`의 `abstention`은 실제 값이 있는데 unknown으로 답한 경우다. 답이 원래 없는 질문에서 unknown은 정답이다. 첫 실행의 숫자 대신 한글 수사를 답한 경우는 `numeric_format_only`로 보조 분류하되 사전 정답을 바꾸지 않았다.

## 재현

보관한 자료의 재분석에는 provider 호출이 없다.

```sh
docs/experiments/real-context-replay/run.sh verify
docs/experiments/real-context-replay/run.sh analyze
```

새 수집은 인증된 Claude·Codex CLI와 원본 출처 파일 또는 보관한 `cases.json`이 필요하다. 원응답을 덮어쓰지 않도록 새로운 출력 폴더를 지정한다. CLI 호출에는 실제 사용량이 발생한다.

```sh
SATURN_REPLAY_HOME="$PWD/.local/experiments/real-context-replay/new-run" \
  docs/experiments/real-context-replay/run.sh collect
```

원래 출처 파일이 없으면 보관한 `corrected/cases.json`을 새 출력 폴더에 먼저 복사하면 같은 재생 입력을 사용할 수 있다. `01-prepare.py`는 기존 출처 목록과 가리기 함수를 사용하므로 출처 목록이 없는 환경에서 원문을 다시 추출하지 않는다. 원문 준비용 의존 코드도 보관 묶음에 포함한다. 패킷 생성은 현재 Rust 소스로 다시 이루어진다. 실험 시점 코드와 현재 코드가 다르면 새 실행으로 구분해야 한다.

Rust 재생 진입점은 private corpus를 명시했을 때만 실행하는 ignored test다. 기본 CI에는 개인 대화가 들어가지 않는다. 외부 모델 응답은 샘플링 때문에 재수집하면 달라질 수 있다.
