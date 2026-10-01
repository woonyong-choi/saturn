# 결정 기록

설계를 정한 이유와 버린 선택지를 결정한 순서대로 남긴다.

| 날짜 | 결정 | 상태 |
|---|---|---|
| 2026-09-29 | [compaction 기준을 절대 토큰 예산으로 정한다](2026-09-29-absolute-token-budget.md) | 채택 |
| 2026-09-29 | [engine만 judge를 부르고 자식 프로세스 환경에서 judge 키를 지운다](2026-09-29-engine-as-judge-proxy.md) | 채택 |
| 2026-09-29 | [engine을 상주 프로세스로 두고 TUI는 JSON-RPC로 붙는 클라이언트로 만든다](2026-09-29-engine-centered-json-rpc.md) | 채택 |
| 2026-09-29 | [판단 기록은 로컬에 쌓고 동의한 레코드만 서버로 올린다](2026-09-29-local-first-judgment-collection.md) | 채택 |
| 2026-09-29 | [provider 설정과 subagent 사용은 사용자 설정을 따르고 Saturn은 추적만 한다](2026-09-29-minimal-provider-control.md) | 채택 |
| 2026-09-29 | [provider마다 입력을 계속 받는 상시 연결을 만든다](2026-09-29-persistent-provider-connections.md) | 채택 |
| 2026-09-29 | [외부 효과가 없음을 증명할 때만 크래시 뒤 자동으로 이어 간다](2026-09-29-proof-based-auto-resume.md) | 채택 |
| 2026-09-29 | [TUI를 포함한 모든 구성 요소를 Rust로 만든다](2026-09-29-rust-for-all-components.md) | 채택 |
| 2026-09-29 | [저장소 하나에 제품별 폴더로 구성 요소를 둔다](2026-09-29-single-repository.md) | 채택 |
| 2026-09-29 | [쓰기 에이전트는 기본으로 한 번에 하나만 실행한다](2026-09-29-single-writer-default.md) | 채택 |
| 2026-09-29 | [판단 규격은 Saturn이 정하고 judge는 중립 이름과 출처로 기록한다](2026-09-29-vendor-neutral-judge-spec.md) | 채택 |
| 2026-10-01 | [맥락 정리는 Saturn 방식을 기본으로 두고 provider 압축 모드를 설정으로 연다](2026-10-01-context-mode-setting.md) | 채택 |
| 2026-10-01 | [관련 항목 후보는 코드 순위로 좁힌 뒤 judge가 고른다](2026-10-01-ranked-candidates-before-judge.md) | 채택 |
| 2026-10-01 | [후보 순위는 임베딩 없이 단어 기반과 용어 카탈로그로 시작한다](2026-10-01-lexical-ranking-with-term-catalog.md) | 채택 |
