# TUI를 포함한 모든 구성 요소를 Rust로 만든다

| 항목 | 값 |
|---|---|
| 날짜 | 2026-09-29 |
| 상태 | 채택 |
| 대체 | 없음 |
| 관련 문서 | [아키텍처](../architecture.md), [TUI](../design/tui.md) |

## 배경

Saturn은 provider 연결을 맡는 engine과 전체 화면을 그리는 TUI로 이루어진다. 구성 요소 중 TUI가 가장 큰 작업이다. TUI의 요구는 Codex처럼 가벼운 전체 화면이다. 대화 없는 대기 상태의 메모리는 Codex 0.158.0 본체가 약 55MB, Claude Code 2.1.284가 약 267MB다(2026-09-29 MacBook Air 측정). Codex TUI는 Rust의 `ratatui`와 `crossterm`으로 만들어져 있고 Apache-2.0으로 공개되어 있다. 판단 모델 학습은 MLX를 쓰기 때문에 Python이 필요하다.

## 선택지

| 선택지 | 장점 | 단점 |
|---|---|---|
| 모든 구성 요소 Rust, 학습만 Python | 네이티브 실행 파일, Codex TUI 코드와 `ratatui` 본보기 활용, 한 언어 관리 | engine과 TUI 모두 비동기 Rust로 작성 |
| Kotlin/JVM과 JLine TUI | JVM 라이브러리 생태계 활용 | JVM 런타임 동반, TUI 라이브러리 비교 조사와 GraalVM 네이티브 검토 필요 |
| TUI만 Rust, engine은 다른 언어 | engine 언어의 별도 선택 | 두 언어 관리 비용, engine 프로세스 메모리 추가 |

## 결정

모든 구성 요소 Rust, 학습만 Python인 방식을 쓴다. Codex 수준의 가벼운 TUI, 한 언어 관리를 가장 중요하게 본다.

## 결과

- engine과 TUI의 타입, 라이브러리 공유
- Codex 코드 재사용 가능, 재사용 때 Apache-2.0 고지 유지
- 학습용 Python 환경 별도 관리

## 다시 볼 조건

- Saturn TUI의 대기 메모리가 Codex 본체 측정값 초과
- 데스크톱 앱 기술 결정에서 Rust 밖 화면 코드 공유 요구 발생
