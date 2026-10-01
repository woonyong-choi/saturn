# 관련 항목 후보는 코드 순위로 좁힌 뒤 judge가 고른다

| 항목 | 값 |
|---|---|
| 날짜 | 2026-10-01 |
| 상태 | 채택 |
| 대체 | 없음 |
| 관련 문서 | [맥락 고르기](../design/context-selection.md), [맥락 정리](../design/context-management.md), [judge](../design/judge.md) |

## 배경

패킷과 결과 전달에서 관련 항목을 고를 때 judge가 후보 전체를 읽으면 요청이 후보 수에 비례해 커진다. judge는 관련 없는 큰 state를 방해 요소로 받고, 요청당 64K와 state와 가장 긴 질문의 합 32K 한도가 있다(2026-09-29 확인). judge가 답하지 못하면 `compact` 대체 규칙은 최근 3턴과 고정 항목만 넣는다. RRF(Reciprocal Rank Fusion)는 여러 순위를 `Σ 1/(k + r)`로 합치고, 원 논문은 TREC 결과와 LETOR 3에서 개별 시스템과 Condorcet Fuse보다 나은 결과를 보였으며 k = 60이 평균적으로 가장 좋았다(Cormack, Clarke, Büttcher, SIGIR 2009). 한국어 검색에서 n-gram 색인은 사전 없이 복합명사를 다루고 형태소 기반 색인보다 효과가 좋았다(Lee, Cho, Park, Information Processing & Management 1999).

## 선택지

| 선택지 | 장점 | 단점 |
|---|---|---|
| RRF로 순위만 합친 뒤 상위 N개만 judge | 단위 무관, 학습 불필요, judge 요청 크기 고정, judge 실패 시 순위로 대체 | k와 N 실측 필요, 코드 채널의 동의어와 번역어 누락 |
| judge가 후보 전체를 판단 | 단순함, 모든 후보에 뜻 판단 | 후보 수에 비례하는 요청, judge 실패 시 큰 맥락 손실 |
| 채널 점수의 가중합 | 직관적 | 단위가 다른 점수의 가중치 학습 필요 |

## 결정

RRF로 순위만 합친 뒤 상위 N개만 judge에 묻는 방식을 쓴다. judge 실패 시 잃는 맥락의 양과 judge 요청 크기의 고정을 가장 중요하게 본다.

## 결과

- 파일 겹침, 단어 겹침, 최근성 세 채널의 후보 순위
- 한글 글자 2개 단위, 영문 식별자 단위의 단어 조각
- judge 실패 시 RRF 상위 N개 사용
- 용어 대응표 관리
- `k`, N, 단어 조각 단위의 실측 필요

## 다시 볼 조건

- [후보 순위의 k와 judge 상위 N 측정](https://github.com/woonyong-choi/saturn/issues/116)에서 judge 전체 판단이 남긴 항목이 상위 N 밖으로 자주 밀림
- [후보 순위와 judge 결합의 전환 품질 측정](https://github.com/woonyong-choi/saturn/issues/117)에서 RRF 상위 N + judge의 전환 품질이 judge 단독보다 낮음
