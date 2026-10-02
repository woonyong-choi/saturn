# 용어

이 프로젝트에서만 쓰는 말이다. provider, session, subagent, TUI처럼 업계에서 널리 쓰는 말은 넣지 않는다.

| 용어 | 뜻 | 원어와 코드 이름 |
|---|---|---|
| 경쟁 구역 | 패킷에서 남은 예산 안에 고른 순서대로 넣는 도구 결과, 다른 에이전트 결과, 파일 경로의 자리다. | competitive zone, `ranked` |
| 고정 구역 | 패킷에서 제약, 목표와 마지막 입력, 미완 항목, 최근 대화를 모두 넣는 자리다. | fixed zone, `fixed` |
| 기록 번호 | 채팅 트리 전체 기록에 매긴 하나의 연속 번호다. | `ledger seq` |
| 기본 폴더 | 채팅을 만든 폴더다. 폴더 설정과 작업 폴더는 이 폴더 하나를 따른다. | default folder |
| 기준값 | judge가 낸 확률로 행동할지 가르는 값이다. | threshold, `threshold` |
| 끼워 넣기 | 진행 중인 턴에 입력을 더하는 처리다. | steer, `steer` |
| 대기 | 보내기 전 채팅 대기열에 있는 입력의 상태다. | `queued` |
| 더한 폴더 | `--add-dir`나 `/add-dir`로 채팅에 더한 기본 폴더 밖의 폴더다. 모든 provider session에 넘기고 폴더 설정은 읽지 않는다. | added folder |
| 메인 에이전트 | 채팅마다 하나인 주 에이전트다. | main agent, `main` |
| 보류 | 사용자가 멈춘 작업과 그때 보내지 않은 입력의 상태다. 자동으로 이어 가지 않는다. | `held` |
| 보조 에이전트 | judge가 하던 일과 무관한 작업으로 판단해 같은 채팅 안에 따로 띄운 에이전트다. provider가 스스로 띄운 subagent와 다르다. | `sub` |
| 작업 | 채팅 안에서 한 에이전트가 맡은 일 하나다. 화면은 작업마다 이름표 A, B, C를 붙인다. | task, `task` |
| 정리 모드 | 맥락 정리를 Saturn과 provider 중 누가 맡을지 정하는 설정이다. | context mode, `context.mode` |
| 질문 세트 | judge에 한 번에 묻는 질문 묶음과 그 버전이다. | question set, `question set` |
| 채점 | 판단 기록에 학습용 정답 라벨을 붙이는 처리다. | labeling, `labeling` |
| 채팅 | 사용자가 보는 대화 하나다. 여러 provider session이 한 채팅에 이어진다. | chat, `chat` |
| 트리 유휴 | 에이전트와 그 아래 모든 subagent가 끝난 상태다. | tree idle, `tree idle` |
| 판단 방식 | 행동을 정하는 judge를 고르는 설정이다. | method, `method` |
| 패킷 | 새 session에 넘기는 맥락 묶음이다. Saturn 기록 원문에서 고른다. | handoff packet, `packet` |
| 효과 범위 | 크래시 뒤 자동으로 이어 가도 되는지 가르는 실행 기록 값이다. | `effect_scope` |
| 후보 순위 | 고를 후보에 파일 겹침, 단어 겹침, 최근성 순위를 매겨 RRF로 합친 순서다. | candidate ranking, `rank` |
| judge | 입력마다 뜻을 확률로 판단하는 작은 모델이다. 외부 API인 기준 judge와 로컬의 Saturn 모델이 있다. | `judge` |
| Saturn 모델 | 판단 기록으로 학습한 로컬 judge다. | student model, `student` |
