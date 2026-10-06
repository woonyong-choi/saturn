# 맥락 복구 순효과: 실험 결과

## 요약

사전 등록한 40회 중 40회의 원자료를 수집했다. 입력 규약 위반은 8회였다. 실제 접수는 676개로 계획 상한 640개를 넘었다. 이 표본은 실패 원인을 구분하는 진단이며 제품 채택을 입증하지 않는다. 원문 조회와 Jev의 실제 조건별 성과는 아래 표와 [집계](results/summary.json)에 있다.

## 방법

| 항목 | 값 |
|---|---|
| 설계 | [실험 설계](design.md), 수집 전 최종 기준 fe0e5b2 |
| 실행 환경 | [env.json](env.json), 제품 기준 main 7413264 |
| 표본 | 신규 시드 101~104, 두 provider, 다섯 조건 |
| 원자료 | 로컬 보존, 공개 파일별 해시는 [SHA256SUMS](data/SHA256SUMS) |

## 설계와 다른 점

사전 설계의 조건·시드·판정 기준은 formal 수집 뒤 변경하지 않았다. 예비 실행은 별도 보존하고 formal에서 제외했다. 분석기에 전체 40회 수집 확인 게이트를 추가했다. 수집 도중 rescue의 여러 줄 입력이 CLI에서 별도 입력으로 접수되는 문제가 발견됐다. F2 채점은 순서 대신 원래 질문과 일치하는 입력 ID로 수정했다. 원자료와 원래 실행은 그대로 보존하고 rescue는 의도한 단일 입력 보강 비교에서 무효로 표시한다. 가설과 채택 기준은 바꾸지 않았다. 사전 설계 문서의 금지 낱말만 같은 뜻으로 교정했다. 기본 압축 호출의 사용량이 0 또는 이전 누적값으로 보고되는 경우가 확인돼 그 값은 압축 비용 미계측으로 분류했다. 관측 가중 비용은 따로 남기고 전체 비용 판정은 보류했다.

## 결과

### 흐름

| provider | 조건 | 수집 | 전체 성공 [Wilson 95%] | 미완료 | 미개입 | 사용량 결측 | 입력 규약 위반 |
|---|---|---:|---:|---:|---:|---:|---:|
| claude | jev_lookup | 4 | 0/4 [0.0, 49.0]% | 1 | 0 | 0 | 0 |
| claude | provider | 4 | 3/4 [30.1, 95.4]% | 1 | 0 | 4 | 0 |
| claude | rescue | 4 | 1/4 [4.6, 69.9]% | 2 | 0 | 2 | 4 |
| claude | rrf | 4 | 0/4 [0.0, 49.0]% | 0 | 0 | 0 | 0 |
| claude | rrf_lookup | 4 | 0/4 [0.0, 49.0]% | 0 | 0 | 0 | 0 |
| codex | jev_lookup | 4 | 0/4 [0.0, 49.0]% | 0 | 0 | 0 | 0 |
| codex | provider | 4 | 4/4 [51.0, 100.0]% | 0 | 0 | 4 | 0 |
| codex | rescue | 4 | 2/4 [15.0, 85.0]% | 2 | 0 | 0 | 4 |
| codex | rrf | 4 | 0/4 [0.0, 49.0]% | 0 | 0 | 0 | 0 |
| codex | rrf_lookup | 4 | 0/4 [0.0, 49.0]% | 0 | 0 | 0 | 0 |

| provider | 조건 | F1 | F2 | F3 | 최종 헤더 보존 |
|---|---|---:|---:|---:|---:|
| claude | jev_lookup | 0/4 | 3/4 | 0/4 | 0/4 |
| claude | provider | 3/4 | 3/4 | 3/4 | 3/4 |
| claude | rescue | 1/4 | 2/4 | 1/4 | 2/4 |
| claude | rrf | 0/4 | 4/4 | 0/4 | 0/4 |
| claude | rrf_lookup | 0/4 | 4/4 | 0/4 | 0/4 |
| codex | jev_lookup | 0/4 | 4/4 | 0/4 | 0/4 |
| codex | provider | 4/4 | 4/4 | 4/4 | 4/4 |
| codex | rescue | 2/4 | 2/4 | 2/4 | 2/4 |
| codex | rrf | 0/4 | 4/4 | 0/4 | 0/4 |
| codex | rrf_lookup | 0/4 | 4/4 | 0/4 | 0/4 |

### 확인 분석

가중 토큰은 실제 토큰 개수나 청구 금액이 아니다. cache read 0.1, Claude cache write 1.25, 출력 5, router 원시 토큰 1의 가정이다. 미완료 실행은 종료까지 발생한 비용이며 작업을 끝낸 비용으로 해석하지 않는다. 기본 압축 작업의 사용량이 계측되지 않은 경우 아래 관측 합계에는 그 비용이 빠져 있다. 전체 비용 비교 표에서는 결측으로 표시한다.

| provider | 조건 | 보고된 가중 비용 평균 | 보고된 원시 토큰 합 평균 | 경계 뒤 초 평균 | 조회 합계 |
|---|---|---:|---:|---:|---:|
| claude | jev_lookup | 81,415.4 | 284,807.5 | 48.9 | 0 |
| claude | provider | 51,733.1 | 246,490.8 | 75.0 | 0 |
| claude | rescue | 147,490.7 | 732,539.5 | 101.6 | 0 |
| claude | rrf | 80,104.8 | 347,113.2 | 51.9 | 0 |
| claude | rrf_lookup | 79,997.5 | 331,033.8 | 46.1 | 0 |
| codex | jev_lookup | 113,241.2 | 467,193.2 | 91.3 | 0 |
| codex | provider | 82,626.1 | 340,486.2 | 78.3 | 0 |
| codex | rescue | 112,882.3 | 481,991.8 | 84.2 | 0 |
| codex | rrf | 77,959.6 | 368,004.8 | 57.5 | 0 |
| codex | rrf_lookup | 87,554.6 | 388,389.2 | 70.2 | 0 |

| provider | 비교 | 품질 차이 [95% 구간] | X만/Y만 성공 | 가중 비용 차이 [95% 구간] | 판정 |
|---|---|---|---|---|---|
| claude | rrf_lookup − provider | -75.0%p [-99.7, 51.3] | 0/3 | 결측 | 보류 |
| claude | jev_lookup − provider | -75.0%p [-99.7, 51.3] | 0/3 | 결측 | 보류 |
| claude | rrf_lookup − rrf | 0.0%p [-66.6, 66.6] | 0/0 | -107.3 [-8,119.3, 5,148.6] | 보류 |
| claude | jev_lookup − rrf_lookup | 0.0%p [-66.6, 66.6] | 0/0 | 1,417.9 [-24,816.9, 16,242.8] | 보류 |
| claude | rescue − rrf | 25.0%p [-66.2, 84.8] | 1/0 | 171,231.0 [126,419.9, 216,042.2] | 실행 규약 위반: 진단 비교 불가 |
| codex | rrf_lookup − provider | -100.0%p [-100.0, 33.1] | 0/4 | 결측 | 보류 |
| codex | jev_lookup − provider | -100.0%p [-100.0, 33.1] | 0/4 | 결측 | 보류 |
| codex | rrf_lookup − rrf | 0.0%p [-66.6, 66.6] | 0/0 | 9,595.1 [-28,831.6, 48,021.8] | 보류 |
| codex | jev_lookup − rrf_lookup | 0.0%p [-66.6, 66.6] | 0/0 | 25,686.6 [-19,191.4, 62,009.9] | 보류 |
| codex | rescue − rrf | 50.0%p [-61.9, 95.3] | 2/0 | 34,922.8 [15,740.0, 66,129.3] | 실행 규약 위반: 진단 비교 불가 |

### 실패한 실행

- claude-102-provider: s1:ok,s2:ok,s3:ok,s4:ok,s5:ok,s6:ok,s7:ok,load0:ok,load1:ok,load2:ok,load3:ok,boundary:ok,compact:ok,f1:permission_wait
- claude-102-rescue: s1:ok,s2:ok,s3:ok,s4:ok,s5:ok,s6:ok,s7:ok,load0:ok,load1:ok,load2:ok,load3:ok,boundary:ok,f1:permission_wait
- claude-103-jev_lookup: s1:ok,s2:ok,s3:ok,s4:ok,s5:ok,s6:ok,s7:ok,load0:ok,load1:ok,load2:ok,load3:ok,boundary:ok,f1:permission_wait
- claude-104-rescue: s1:ok,s2:ok,s3:ok,s4:ok,s5:ok,s6:ok,s7:ok,load0:ok,load1:ok,load2:ok,load3:ok,boundary:ok,f1:permission_wait
- codex-103-rescue: s1:ok,s2:ok,s3:ok,s4:ok,s5:ok,s6:ok,s7:ok,load0:ok,load1:ok,load2:ok,load3:ok,boundary:ok,f1:permission_wait
- codex-104-rescue: s1:ok,s2:ok,s3:ok,s4:ok,s5:ok,s6:ok,s7:ok,load0:ok,load1:ok,load2:ok,load3:ok,boundary:ok,f1:permission_wait

## 논의

### 해석

rescue의 여러 줄 보강은 실제로 여러 입력과 실행으로 나뉘었다. 표의 값은 발생한 동작을 기록한 것이며 사전 설계한 단일 입력 보강의 효과로 해석하지 않는다. 이 조건은 원인 진단 비교에도 사용할 수 없다.

조회 설정을 켠 것과 실제로 조회해서 근거를 되찾은 것을 구분한다. 조회 횟수와 개별 결과는 trials.json에 있다. 본문의 성공률은 중단까지 포함한 실제 작업 완료율이며 모델의 기억 능력만의 점수가 아니다.

### 타당성 위협

| 종류 | 위협 | 이 실험에서 |
|---|---|---|
| 내적 | 모델 초기 응답과 서버·cache 상태가 다르다. | 입력과 시작 저장소는 같고 순서를 섞었지만 동일한 provider 내부 상태를 복제한 실험은 아니다. |
| 구성 | 원시 토큰과 가격이 다르다. | 비용은 가중 지표로 한정하며 실제 요금 절감을 주장하지 않는다. |
| 구성 | 승인 대기·중단은 품질과 비용에 동시에 영향을 준다. | 실행 실패를 보존하며 중단된 작업의 낮은 비용을 효율 개선으로 해석하지 않는다. |
| 외적 | 한 종류의 합성 서비스와 네 시드다. | 실제 프로젝트와 다른 모델·기본 압축 시점에 일반화하지 않는다. |

### 한계

- 같은 session 내부의 과거 기록 편집은 측정하지 않았다.
- 품질 저하가 없음을 일반적으로 입증할 표본 크기가 아니다.
- 원문 조회를 켠 결과만으로 Jev 선별 일관성 개선을 입증하지 않는다.
- 품질 구간은 두 불일치 비율에 Clopper–Pearson 정확 이항 구간을 구하고 Bonferroni 경계를 적용한 보수적인 구간이다. 이항 구간의 정의는 [SciPy 공식 문서](https://docs.scipy.org/doc/scipy/reference/generated/scipy.stats._result_classes.BinomTestResult.proportion_ci.html)를 따른다. 작은 표본에서 구간이 넓다는 사실을 숨기지 않는다.

## 재현

```sh
./run.sh analyze
./run.sh verify
```

원자료가 없는 환경에서는 verify를 통과했다고 표시하지 않는다. 기존 context-net-effect의 검증은 기록된 Python 3.13에서 재집계·분석·반복 판단 모두 바이트 단위로 일치했다. Python 3.9에서는 소수점 말단과 반올림 차이가 있어 같은 바이트가 아니었다.

## 결론

이 진단으로 제품 자동 적용을 승인하지 않는다. 조건별 기각·보류와 실제 실패 경로를 보존하고, 후속 설계는 원문 손실·조회 실패·추가 비용을 구분한 근거에 한정한다. provider 기본 압축을 대체한다는 목표의 달성 여부와 진단 조건의 성공을 동일하게 표시하지 않는다.
