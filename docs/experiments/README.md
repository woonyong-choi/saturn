# 실험

| 실험 | 확인할 것 | 관련 설계 | 결론 |
|---|---|---|---|
| [rrf-k-top-n](rrf-k-top-n/design.md) | 후보 순위의 k와 judge 상위 N | [맥락 고르기](../design/context-selection.md) | 측정 전 |
| [precompute-breakeven](precompute-breakeven/design.md) | 도구 결과 미리 판단의 손익분기 | [맥락 정리](../design/context-management.md) | 측정 전 |
| [claude-summary-handoff](claude-summary-handoff/design.md) | Claude 압축 요약 전환 품질 | [맥락 정리](../design/context-management.md), [provider 연결과 session](../design/providers-and-sessions.md) | 측정 전 |
| [fast-adjust-convergence](fast-adjust-convergence/design.md) | 빠른 조정의 수렴과 진동 | [judge 학습](../design/judge-training.md) | 측정 전 |
| [ranked-handoff-quality](ranked-handoff-quality/design.md) | 후보 순위와 judge 결합의 전환 품질 | [맥락 고르기](../design/context-selection.md) | 측정 전 |
| [wordpiece-typo-recall](wordpiece-typo-recall/design.md) | 단어 조각 단위별 오타 재현율 | [맥락 고르기](../design/context-selection.md) | 측정 전 |
| [term-catalog-precision](term-catalog-precision/report.md) | 용어 카탈로그 짝 정밀도와 묶음 확인 비용 | [용어 카탈로그](../design/term-catalog.md) | H1 기각: 괄호 표기 정밀도 1.5% [0.5, 4.3] |
| [constraint-judge-accuracy](constraint-judge-accuracy/design.md) | 제약 식별과 대체 판정 정확도 | [맥락 고르기](../design/context-selection.md), [judge](../design/judge.md) | 측정 전 |
