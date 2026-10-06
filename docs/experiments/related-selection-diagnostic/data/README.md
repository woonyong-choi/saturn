# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | 합성 파일 12개씩을 실제 Codex 도구 호출로 기록한 뒤 동일 기록을 `off`·`rank`·`jev`의 Claude Sonnet 새 세션에 복제 |
| 수집 기간 | 2026-10-07 |
| 개수 | 예정 12계열·36 target, source 전문 확보 10계열·실행 30 target |
| 표본 여부 | 네 종류 x 씨앗 세 개를 수집 전에 고정한 개발 진단 |
| 라벨 | 생성 시 정한 현재 정수 값. 숨긴 target 답과 정확히 비교 |
| 알려진 문제 | 두 source에서 Codex 완료 상태에도 파일 전문을 담은 ToolResult가 없었음 |
| 개인정보 | 합성문만 사용. 원응답은 Git 제외 로컬 실험 폴더에 보존 |
| 라이선스 | 프로젝트 내부 합성 fixture |

## 파일

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| 비공개 `manifest.json` | 실행 ID·엔진 해시·계열 설정 | `scripts/01-collect.py` |
| 비공개 `{계열}/source.json` | 실제 Codex 기록과 전문 확보 여부 | `scripts/01-collect.py` |
| 비공개 `{계열}/{조건}.json` | 실제 target 응답·패킷·판단·조회·usage | `scripts/01-collect.py` |
| [trials.csv](processed/trials.csv) | 조건별 채점과 집계 입력 | `scripts/02-process.py` |
| [SHA256SUMS](SHA256SUMS) | 비공개 원자료의 SHA-256 | `scripts/02-process.py` |

## 필드

### `{계열}/{조건}.json`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `case` | string | 없음 | 계열 고유 | 종류·씨앗 | `middle-0` |
| `arm` | string | 없음 | `off`, `rank`, `jev` | 선별 조건 | `jev` |
| `status` | string | 없음 | 필수 | target 실행 상태 | `ok` |
| `answer` | string | 없음 | 실패 시 빈 값 | target 마지막 텍스트 | `123` |
| `expected` | integer | 없음 | 필수 | 생성 때 확정한 정답 | `123` |
| `db` | object | 없음 | 필수 | 패킷·항목·판단·usage·조회 행 | `{"packets":[]}` |

로컬 실행 ID는 `20261006T210043Z-6b4ccf7`이다. 실험 바이너리 SHA-256은 `b1de516ad87c4c43fa4f189102bbcfd6868cfbe7421844bf0d531811055dca86`다. 저장소의 Git 제외 실험 폴더가 있는 환경에서 `./run.sh verify 20261006T210043Z-6b4ccf7`로 재검증한다.
