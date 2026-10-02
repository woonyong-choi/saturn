# 판단 규격은 Saturn이 정하고 router는 중립 이름과 출처로 기록한다

| 항목 | 값 |
|---|---|
| 날짜 | 2026-09-29 |
| 상태 | 채택 |
| 대체 | 없음 |
| 관련 문서 | [아키텍처](../architecture.md), [router](../design/router.md), [router 학습](../design/router-training.md) |

## 배경

첫 router는 외부 판단 API인 TypeSafe Jev다. 목표는 판단 기록으로 학습한 Saturn 모델로 router를 바꾸는 것이다. Jev가 공개한 것은 `choice`, `noul`, `score` 형식의 입출력 규격이다(2026-09-29 확인). 판단 기록은 채점을 거쳐 Saturn 모델의 학습 데이터로 쓴다. 기록 안에는 외부 서비스의 출력과 Saturn 자체의 출력이 섞인다.

## 선택지

| 선택지 | 장점 | 단점 |
|---|---|---|
| Saturn 판단 규격, 중립 `router_id`, 출처 기록 | router 교체 때 스키마와 코드 유지, 외부 출력이 섞인 데이터의 선별 제외 | 규격 정의와 `router_id` 매핑 관리 |
| router 벤더 형식과 이름 그대로 사용 | 벤더 API 직접 호출, 변환 불필요 | router 교체 때 스키마와 코드 수정, 데이터 출처 구분 불가 |

## 결정

Saturn 판단 규격, 중립 `router_id`, 출처 기록을 쓴다. router 교체 가능성, 학습 데이터의 출처 추적을 가장 중요하게 본다.

## 결과

- 모든 router를 `RouterClient` 구현체로 연결
- 실제 모델과 버전은 설정의 `router_id` 매핑과 `router_manifest`에만 저장
- 판단과 라벨마다 출처와 사용 제한 기록
- 판단 규격의 직접 정의와 유지

## 다시 볼 조건

- router 제품 사이 공통 판단 규격 등장
- Saturn 규격으로 표현하지 못하는 판단 형식 필요
