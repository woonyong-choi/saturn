# 문서

Saturn의 설계 문서다. 문서는 한국어로 쓴다. 처음이면 아키텍처부터 읽는다.

| 문서 | 내용 |
|---|---|
| [아키텍처](architecture.md) | 구성 요소, 코드 지도, 불변 조건 |
| [입력 처리](design/input-handling.md) | 입력 접수, 실행 중 새 입력, 대기, 보류와 재개, 쓰기 규칙 |
| [provider 연결과 session](design/providers-and-sessions.md) | provider 계층과 어댑터, provider 연결, session 수명과 전환, subagent 추적, 사용량 |
| [기능 목록과 확장](design/extensions.md) | provider 기능 목록, 확장 저장소, 설치와 주입, 옮길 수 없는 부분 알림 |
| [권한](design/permissions.md) | 권한 규칙, 항상 허용 저장, Codex와 Claude 구성, 허가 대기 중 피드백 |
| [입력 요청](design/input-requests.md) | provider 입력 요청 세 형식, 공통 입력 요청과 답, 대기와 보관, 에이전트 질문 설정 |
| [맥락 정리](design/context-management.md) | 맥락 크기 측정, 정리 판정, 정리 모드, 패킷 구성 |
| [맥락 고르기](design/context-selection.md) | 후보 순위, 단어 조각, 도구 결과 메모, compact 요청 |
| [제약](design/constraints.md) | 제약 식별, 저장, 해제와 예외, 묻기와 되돌리기, 패킷 제약 칸 |
| [router](design/router.md) | 판단 질문, 답 형식, 기준값과 대체 규칙, 판단 기록 |
| [router 키 보호](design/router-key-security.md) | router 키 입력, 저장, 자식 프로세스 차단 |
| [router 학습](design/router-training.md) | 기준값 조정, 채점, 로컬 모델 승격 |
| [설정](design/settings.md) | 설정 층, 폴더 설정 신뢰, 설정 번호 |
| [기록 저장과 보존](design/records.md) | 기록 저장소, 스키마 이관, 보존과 삭제 |
| [하위 접속](design/child-sessions.md) | 에이전트 안팎에서 온 접속, 출입증, 상한과 대기열, 부모와 함께 끝나기, 부하 시험 |
| [engine 수명과 복구](design/engine-lifecycle.md) | engine 시작, TUI 종료 뒤 계속, 크래시 뒤 복구, 업데이트로 engine 교체, 채팅 폴더와 이어 열기 |
| [TUI](design/tui.md) | 화면 배치, 키, 상태 표시 |
| [용어](glossary.md) | 이 프로젝트에서만 쓰는 말 |
| [결정 기록](decisions/README.md) | 설계를 정한 이유와 버린 선택지 |
| [실험](experiments/README.md) | 설계 값을 확인하는 실험과 결론 |
| [같은 세션 축약 비교](experiments/same-session-compaction/report.md) | 같은 프로세스의 기본·fast-jev 축약과 새 패킷 인수 결과 |
| [패킷 인수 순서 검증](experiments/same-session-compaction/handoff-design.md) | 패킷 인수와 후속 질문을 나눈 추가 실험 설계 |
| [프로세스 유지 축약 검증](experiments/same-session-compaction/persistent-design.md) | 축약과 후속 질문 사이에 CLI 연결을 유지한 추가 실험 설계 |
