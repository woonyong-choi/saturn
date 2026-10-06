# 데이터

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| [SHA256SUMS](SHA256SUMS) | 비공개 원응답 288개의 SHA-256과 비공개 폴더 기준 이름 | scripts/02-process.py |
| [집계](../results/summary.json) | 조건·구간별 성공, 적용률, 사용량, 시간 | scripts/03-analyze.py |

원응답(요청, 응답, 사용량, 시각)은 저장소의 비공개 실험 공간 `.local/experiments/arm-execution-verifier/live`에 둔다. 원자료 접근 권한이 있는 환경에서 `ARM_VERIFIER_CALLER=live run.sh process`, `analyze`, `verify`로 재현한다. 모델 재호출의 응답과 시간까지 재현되지는 않는다. 표본은 새 합성 과제이며 사용자 대화와 API 키를 포함하지 않는다.
