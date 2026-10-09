# 질문별 원문 복원 자료

| 자료 | 위치 |
|---|---|
| 실제 수집 | 저장소의 `.local/experiments/context-recall/` |
| 최초 수집과 중단 | `confirmation/`, `stopped.json` 및 남은 JSONL |
| 종류별 배분 비교 | `balanced/` |
| 최종 경계 비교 | `bounded/` |
| 추가 반복 | `replication/` |
| 탐색과 변환 증거 | `development/`, `heldout-*.json`, `heldout-reproduced/` |
| 최종 구현과 기존 변경 구분 | `final-source/` |
| 실제 화면과 종료 | `tui-final/` |
| 검사 로그 | `.local/verification/context-recall/` |
| 보관 파일·해시 | 이 폴더의 `archive.json`, `SHA256SUMS` |

`raw/` 아래 조건별 폴더에는 입력, 질문, 요청 정보, stdout JSONL, stderr, 해석한 결과와 사용량이 있다. 원응답과 부분 출력은 수정하지 않았다. `collection-seal.json`은 수집 전에 고정한 자료와 코드의 해시다. 원본 대화는 마스킹된 사본, 원본 파일 해시와 줄 번호로 추적한다. 인증 정보 파일은 포함하지 않는다.

원본 로그에 접근할 수 있으면 `scripts/00-prepare-heldout.py --output <새 폴더>`로 별도 설계 자료를 재생성할 수 있다. 입력 변환의 동일성을 `heldout-reproducibility.json`에 보존했다. 원본 없이 아카이브만 복원해도 고정된 재생 자료·모델 응답·사용량으로 보고서 수치를 재계산할 수 있다.

검증기는 수집 봉인, 입력과 계획의 일치, 원응답과 해석 결과의 일치, session·도구 호출 정보, 파일별 해시, 압축 파일 해시를 검사한다. 아카이브 생성 시 압축 파일을 다시 읽어 모든 파일의 해시를 대조한다. 공개 연구 문서에는 마스킹 전 원본과 인증 정보를 넣지 않는다.
