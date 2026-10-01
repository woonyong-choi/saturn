# 문서

Saturn의 설계 문서다. 문서는 한국어로 쓴다. 처음이면 아키텍처부터 읽는다.

| 문서 | 내용 |
|---|---|
| [아키텍처](architecture.md) | 구성 요소, 코드 지도, 불변 조건 |
| [입력 처리](design/input-handling.md) | 입력 접수, 실행 중 새 입력, 대기, 보류와 재개, 쓰기 규칙 |
| [provider 연결과 session](design/providers-and-sessions.md) | provider 연결, session 수명과 전환, subagent 추적, 사용량 |
| [맥락 정리](design/context-management.md) | 맥락 크기 측정, 정리 판정, 정리 모드, 패킷 구성 |
| [맥락 고르기](design/context-selection.md) | 후보 순위, 단어 조각, 도구 결과 메모, 제약 식별과 대체 |
| [용어 카탈로그](design/term-catalog.md) | 용어 짝 캐기, 짝 점수, 확인, 층과 승격 |
| [judge](design/judge.md) | 판단 질문, 답 형식, 기준값과 대체 규칙, 판단 기록 |
| [judge 키 보호](design/judge-key-security.md) | judge 키 입력, 저장, 자식 프로세스 차단 |
| [judge 학습](design/judge-training.md) | 기준값 조정, 채점, 로컬 모델 승격 |
| [설정](design/settings.md) | 설정 층, 폴더 설정 신뢰, 설정 번호 |
| [기록 저장과 보존](design/records.md) | 기록 저장소, 스키마 이관, 보존과 삭제 |
| [engine 수명과 복구](design/engine-lifecycle.md) | engine 시작, TUI 종료 뒤 계속, 크래시 뒤 복구 |
| [TUI](design/tui.md) | 화면 배치, 키, 상태 표시 |
| [용어](glossary.md) | 이 프로젝트에서만 쓰는 말 |
| [결정 기록](decisions/README.md) | 설계를 정한 이유와 버린 선택지 |
| [실험](experiments/README.md) | 설계 값과 외부 도구 동작을 잰 실험 |
