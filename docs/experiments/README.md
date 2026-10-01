# 실험

| 실험 | 확인할 것 | 관련 설계 | 결론 |
|---|---|---|---|
| [rrf-k-top-n](rrf-k-top-n/design.md) | 후보 순위의 k와 judge 상위 N | [맥락 고르기](../design/context-selection.md) | 측정 전 |
| [precompute-breakeven](precompute-breakeven/design.md) | 도구 결과 미리 판단의 손익분기 | [맥락 정리](../design/context-management.md) | 측정 전 |
| [claude-summary-handoff](claude-summary-handoff/design.md) | Claude 압축 요약 전환 품질 | [맥락 정리](../design/context-management.md), [provider 연결과 session](../design/providers-and-sessions.md) | 측정 전 |
| [fast-adjust-convergence](fast-adjust-convergence/design.md) | 빠른 조정의 수렴과 진동 | [judge 학습](../design/judge-training.md) | 측정 전 |
| [ranked-handoff-quality](ranked-handoff-quality/design.md) | 후보 순위와 judge 결합의 전환 품질 | [맥락 고르기](../design/context-selection.md) | 측정 전 |
| [wordpiece-typo-recall](wordpiece-typo-recall/report.md) | 단어 조각 단위별 오타 재현율 | [맥락 고르기](../design/context-selection.md) | H1 기각: 재현율 차이 0.9%p [−0.8, 2.6], H2 기각: 정밀도 차이 −10.0%p [−12.5, −7.5], H3 채택: 재현율 차이 11.6%p [8.8, 14.5], H4 기각: 정밀도 차이 −5.2%p [−7.8, −2.7] |
