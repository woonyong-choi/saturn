# Saturn 이슈 #194 후속 실험: 전용 CODEX_HOME 방식의 구멍 막기

저장소: `~/workspace/oss/saturn` (GitHub `woonyong-choi/saturn`). 먼저 `gh issue view 194 --comments`로 앞 실험 다섯 개를 읽어라. 특히 마지막 댓글(전용 `CODEX_HOME` + execpolicy 번역 실험)의 방법을 그대로 쓴다: 전용 home에 원본 `~/.codex/auth.json`을 가리키는 심볼릭 링크, 권한 키를 뺀 `config.toml`, `rules/default.rules`, app-server stdio, `thread/start`의 `approvalPolicy`.

## 목표
앞 실험에서 남은 구멍 네 개를 설정만으로 막아, Saturn이 Codex의 셸 명령, 파일 편집, MCP 도구, subagent 실행을 모두 판단(승인 요청으로 받기)할 수 있는지 확인한다.

## 확인할 것 (각 조건 3회 반복. 3회 모두 같아야 "확인", 다르면 "불안정", 못 하면 "확인 못 함". 추정으로 채우지 마라)
1. **안전 목록 전부 묻기**: Codex 소스에서 "안전하다고 보고 묻지 않고 실행하는 명령" 판정 코드와 목록(예: known safe command 판정 함수, 읽기 전용 명령 목록)을 찾는다(소스 경로·줄). 그 목록의 명령을 전용 home 규칙에 모두 `prompt`로 적고 `approvalPolicy`를 `untrusted`(또는 소스상 맞는 값)로 줬을 때, 목록 명령(`ls`, `cat`, `head`, `wc` 등 3개 이상)과 목록에 없는 무해한 명령(`node -e "console.log(1)"`, `touch <worktree 안 파일>`) 모두 승인 요청으로 오는가. 모델이 명령을 묶어서 실행하면 회차마다 명령 하나씩만 실행하도록 프롬프트를 좁힌다
2. **파일 편집 묻기**: 샌드박스를 읽기 전용(`read-only` 또는 소스상 맞는 값, `thread/start`나 `-c`)으로 띄우면 파일 편집(`apply_patch`)이 `item/fileChange/requestApproval` 같은 승인 요청으로 오는가. 거부하면 파일이 생기지 않는가. 셸 명령의 쓰기(`touch`)도 승인 요청으로 오는가. 읽기 전용 샌드박스에서 일반 작업(읽기, 테스트 실행 같은 무해한 명령)이 지나치게 막히지 않는지도 기록
3. **MCP 묻기**: (a) Codex 설정·소스에서 MCP 도구별 승인 설정(예: 도구 승인 모드, `mcp_servers.<id>` 아래 승인 관련 키)이 있는지 찾고 있으면 시험한다. (b) 없으면 Saturn이 띄우는 로컬 MCP 중계 서버(worktree 안 스크립트, 무해한 도구 하나)를 전용 config에 넣어, 도구 호출이 중계 서버를 거칠 때 중계가 거부하면 실행되지 않는지 확인한다. 사용자 config의 다른 MCP 서버는 전용 config에 넣지 않는다
4. **subagent**: subagent(자식 thread)를 띄우게 하는 프롬프트로 5회 반복해 자식이 실행한 명령이 전용 home 규칙(prompt)으로 승인 요청이 오는지 본다. 자식이 명령을 실행하지 않은 회차는 "실행 없음"으로 따로 센다. 소스에서 자식 thread가 부모의 execpolicy·approvalPolicy·sandbox를 물려받는 코드도 찾는다. subagent 기능을 세션 설정으로 끄는 키(기능 플래그 등)가 있는지도 찾아 기록만 한다(끄는 것은 사용자 결정이 필요하므로 시험은 해도 되지만 권장으로 쓰지 마라)
5. **종합 시나리오**: 1~3의 설정을 모두 함께 켠 한 세션에서 셸 읽기 명령, 셸 쓰기 명령, 파일 편집, MCP 도구를 각각 한 번씩 시키고(3회 반복) 네 가지 모두 승인 요청으로 오는지, 거부하면 모두 막히는지

## 장소와 안전 규칙 (반드시)
- `git -C ~/workspace/oss/saturn worktree add ../saturn.wt/experiment-194-codex-plug -b experiment/194-codex-plug origin/main`. 전용 home, 규칙, MCP 중계 스크립트, 로그, 테스트 파일은 모두 이 worktree 안. 커밋하지 않는다
- 로그인 원본(`~/.codex/auth.json`)은 복사·이동·수정 금지. 심볼릭 링크만. 토큰 값 출력 금지
- 사용자 전역 설정(`~/.codex/config.toml`, `~/.codex/rules/`, `~/.codex/hooks.json`)은 읽기만
- 금지: `/tmp`, `$TMPDIR`, `mktemp`, 저장소 사본, `--dangerously-*` 옵션, 파일 삭제·네트워크·설치 명령. Codex에 실행시키는 명령은 worktree 안 파일에 대한 `touch`, `echo`, `ls`, `cat`, `head`, `wc`, `sort`, `node -e "console.log(1)"`만
- 실험이 띄운 app-server, MCP 중계, codex 프로세스는 끝날 때 모두 종료한다. 다른 세션의 codex 프로세스는 건드리지 마라
- Codex 모델 호출 상한 60회. 상한에 닿으면 그 자리에서 멈추고 거기까지의 결과로 보고한다. 넘기지 마라
- 커밋, PR, 이슈 닫기 금지. AI 흔적을 남기지 마라

## 보고
이슈 #194에 댓글로 남긴다(한국어, 표): 확인할 것 / 회차별 관측 / 결과 / 근거(소스 경로·줄, 문서 URL, 실제로 열어 본 것만). 마지막에 결론: 이 설정 조합으로 Saturn이 Codex의 셸·파일 편집·MCP·subagent 실행을 모두 판단할 수 있는가, 남는 구멍과 조건, 일반 작업 불편(읽기 전용 샌드박스 영향). 같은 내용을 `~/workspace/woon/.local/orchestration/saturn-build/codex-194-plug-report.md`에도 쓴다. 실제 모델 호출 횟수를 적는다.

끝나면 worktree의 실험 파일과 심볼릭 링크를 지우고 `git -C ~/workspace/oss/saturn worktree remove --force ../saturn.wt/experiment-194-codex-plug`, `git -C ~/workspace/oss/saturn branch -D experiment/194-codex-plug`로 정리한다.
