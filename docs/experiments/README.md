# 실험

| 실험 | 확인할 것 | 관련 설계 | 결론 |
|---|---|---|---|
| [rrf-k-top-n](rrf-k-top-n/report.md) | 후보 순위의 k와 judge 상위 N | [맥락 고르기](../design/context-selection.md) | H1 기각: recall 12.0% [9.2, 15.6] |
| [precompute-breakeven](precompute-breakeven/report.md) | 도구 결과 미리 판단의 손익분기 | [맥락 정리](../design/context-management.md) | H2 기각: 남는 질문 감소 29.4% [26.4, 32.3], H3 기각: 토큰 4.967배 [4.337, 5.614] |
| [claude-summary-handoff](claude-summary-handoff/design.md) | Claude 압축 요약 전환 품질 | [맥락 정리](../design/context-management.md), [provider 연결과 session](../design/providers-and-sessions.md) | 측정 전 |
| [fast-adjust-convergence](fast-adjust-convergence/report.md) | 빠른 조정의 수렴과 진동 | [judge 학습](../design/judge-training.md) | H3 기각: 100건 안 기준값 폭 0.051 [0.050, 0.052], H1 채택: 틀림 비율 5.6% [5.5, 5.7] |
| [ranked-handoff-quality](ranked-handoff-quality/design.md) | 후보 순위와 judge 결합의 전환 품질 | [맥락 고르기](../design/context-selection.md) | #117로 대체(#198) |
| [wordpiece-typo-recall](wordpiece-typo-recall/report.md) | 단어 조각 단위별 오타 재현율 | [맥락 고르기](../design/context-selection.md) | H1 기각: 재현율 차이 0.9%p [−0.8, 2.6], H2 기각: 정밀도 차이 −10.0%p [−12.5, −7.5], H3 채택: 재현율 차이 11.6%p [8.8, 14.5], H4 기각: 정밀도 차이 −5.2%p [−7.8, −2.7] |
| [term-catalog-precision](term-catalog-precision/report.md) | 용어 카탈로그 짝 정밀도와 묶음 확인 비용 | [용어 카탈로그](https://github.com/woonyong-choi/saturn/blob/6fda8f610b49d8febe162bf7ec36114513b2e741/docs/design/term-catalog.md) | H1 기각: 괄호 표기 정밀도 1.5% [0.5, 4.3] |
| [constraint-judge-accuracy](constraint-judge-accuracy/report.md) | 제약 식별과 대체 판정 정확도 | [맥락 고르기](../design/context-selection.md), [judge](../design/judge.md) | H1 채택: 정밀도 91.6% [84.8, 95.5], H2 채택: 재현율 93.3% [86.9, 96.7], H3~H6 보류 |
| [embedding-synonym](embedding-synonym/report.md) | 임베딩 같은 뜻 찾기의 정확도와 비용 | [맥락 고르기](../design/context-selection.md), [용어 카탈로그](https://github.com/woonyong-choi/saturn/blob/6fda8f610b49d8febe162bf7ec36114513b2e741/docs/design/term-catalog.md) | H1, H2 기각: 제안 0개, H3, H4 기각: 재현율 0.0% [0.0, 6.0], H5, H6 기각: 재현율 차이 0.0%p [−6.0, 6.0], H7 기각: 재현율 차이 −10.5%p [−13.9, −7.6], H8 기각: 재현율 차이 −28.3%p [−32.9, −23.8], H9 기각: 설치 577.7MB, 메모리 406.0MB, H10 기각: 설치 342.5MB, 메모리 287.4MB |
| [indirect-constraint-accuracy](indirect-constraint-accuracy/report.md) | 보류된 제약 대체 구간과 간접 지시 정확도 | [맥락 고르기](../design/context-selection.md), [judge](../design/judge.md) | H1 보류: 간접 지시 정확도 74.5% [68.0, 80.0], H2~H4 보류, H5 보류: 정확도 차이 +0.5%p [−3.8, 4.8], H6 기각: 정확도 차이 −5.5%p [−10.6, −0.5], H7 채택: 정확도 97.3% [93.3, 99.0], H8 채택: 정확도 96.7% [92.4, 98.6] |
| [record-fidelity](record-fidelity/report.md) | Codex와 Claude 기록 변환의 충실도와 전환 품질 | [provider 연결과 session](../design/providers-and-sessions.md), [맥락 고르기](../design/context-selection.md) | C1 기각: 경로 추출 정확도 차이 −20.0%p [−28.0, −13.1], 메모 필드 정확도 차이 −25.0%p [−31.6, −19.1], C2 보류: 2단계 실행 없음 |
| [handoff-packet-quality](handoff-packet-quality/report.md) | Jev 전체 판단 패킷의 전환 품질과 대체 경로 | [맥락 정리](../design/context-management.md), [맥락 고르기](../design/context-selection.md), [judge](../design/judge.md) | H1 보류: 정답률 차이 +8.4%p [6.1, 10.8], H2 보류: 정답률 차이 +7.0%p [4.5, 9.6] |
| [handoff-packet-quality-v2](handoff-packet-quality-v2/report.md) | 새 패킷 규칙의 전환 품질 재측정 | [맥락 정리](../design/context-management.md), [맥락 고르기](../design/context-selection.md), [judge](../design/judge.md) | H1 채택: 정답률 차이 +51.4%p [45.2, 57.3], H2 기각: 정답률 차이 +50.7%p [44.7, 56.7] |
| [record-fidelity-stage2](record-fidelity-stage2/design.md) | 변환 수정 뒤 Codex와 Claude 기록의 전환 품질 | [provider 연결과 session](../design/providers-and-sessions.md), [맥락 고르기](../design/context-selection.md), [맥락 정리](../design/context-management.md) | 측정 전 |
