# 맥락 정리는 Saturn 방식을 기본으로 두고 provider 압축 모드를 설정으로 연다

| 항목 | 값 |
|---|---|
| 날짜 | 2026-10-01 |
| 상태 | 채택 |
| 대체 | 없음 |
| 관련 문서 | [맥락 정리](../design/context-management.md), [설정](../design/settings.md) |

## 배경

Saturn은 provider 압축 없이 기록 원문으로 패킷을 만들어 맥락 전달 품질을 지킨다. Saturn은 사용자 provider 설정을 우선하고 기본값은 사용자 설정이 없을 때만 쓴다([결정 기록](2026-09-29-minimal-provider-control.md)). provider 압축을 쓰면 provider가 들고 있는 맥락과 Saturn 기록이 달라진다. Codex 원격 압축 요약은 암호화되어 무엇이 남았는지 볼 수 없다.

## 선택지

| 선택지 | 장점 | 단점 |
|---|---|---|
| 기본 Saturn, 설정으로 provider 모드 허용, 전환 때 provider별 전달원 선택 | 사용자 선택 존중, 읽을 수 있는 요약 활용 | 모드별 전달 경로 두 벌, 요약 품질 실측 필요 |
| Saturn 방식만 허용 | 전달 품질과 기록 일치 | provider 압축을 원하는 사용자 설정 무시 |
| provider 압축만 사용 | 구성 단순 | Codex 전환 품질의 보장 수단 부재 |

## 결정

기본 Saturn 방식과 설정으로 여는 provider 모드를 쓴다. 맥락 전달 품질과 사용자 provider 설정 존중을 가장 중요하게 본다.

## 결과

- `context.mode` 설정(`saturn`, `provider`)
- `provider` 모드의 전환 때 떠나는 provider의 압축 요약을 읽을 수 있으면 그 요약, 아니면 Saturn 패킷 전달
- 어느 모드든 제약, 목표, 미완 항목은 Saturn 기록 원문으로 전달
- 정리 모드별 전달 경로 두 벌 유지
- Claude 압축 요약 읽기 경로와 전환 품질 실측 필요

## 다시 볼 조건

- [Claude 압축 요약과 패킷의 전환 품질 측정](https://github.com/woonyong-choi/saturn/issues/122)에서 Claude 압축 요약을 안정적으로 읽을 수 없음
- 같은 전환에서 provider 요약 기반 전달의 전환 뒤 질문 정답률이 Saturn 패킷보다 낮음
