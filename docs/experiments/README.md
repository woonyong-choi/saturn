# 실험

| 실험 | 확인할 것 | 관련 설계 | 결론 |
|---|---|---|---|
| [rrf-k-top-n](rrf-k-top-n/design.md) | 후보 순위의 k와 judge 상위 N | [맥락 고르기](../design/context-selection.md) | 측정 전 |
| [precompute-breakeven](precompute-breakeven/report.md) | 도구 결과 미리 판단의 손익분기 | [맥락 정리](../design/context-management.md) | H2 기각: 남는 질문 감소 29.4% [26.4, 32.3], H3 기각: 토큰 4.967배 [4.337, 5.614] |
| [claude-summary-handoff](claude-summary-handoff/design.md) | Claude 압축 요약 전환 품질 | [맥락 정리](../design/context-management.md), [provider 연결과 session](../design/providers-and-sessions.md) | 측정 전 |
| [fast-adjust-convergence](fast-adjust-convergence/design.md) | 빠른 조정의 수렴과 진동 | [judge 학습](../design/judge-training.md) | 측정 전 |
| [ranked-handoff-quality](ranked-handoff-quality/design.md) | 후보 순위와 judge 결합의 전환 품질 | [맥락 고르기](../design/context-selection.md) | 측정 전 |
| [wordpiece-typo-recall](wordpiece-typo-recall/design.md) | 단어 조각 단위별 오타 재현율 | [맥락 고르기](../design/context-selection.md) | 측정 전 |
| [term-catalog-precision](term-catalog-precision/design.md) | 용어 카탈로그 짝 정밀도와 묶음 확인 비용 | [용어 카탈로그](../design/term-catalog.md) | 측정 전 |
