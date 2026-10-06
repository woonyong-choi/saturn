# 평가 입력 계약

개발 스모크의 봉인된 12개 요청 해시는 [dev-source-lock.json](dev-source-lock.json)에 있다. 원요청과 키는 공개 폴더로 복사하지 않는다.

확인 입력 `fixtures.jsonl`은 #7의 새 J trial 원자료가 나온 뒤 만든다. 각 줄은 `task_id`, `phase`, `project_family`, `source_sha256`, `goal_sha256`, `candidate_sha256`, `policy_sha256`, `compact_request_sha256`, `raw_request`, `k`, `required_ids`, `critical_required_ids`, `requests`를 포함한다. `requests`에는 `identical`, `order`, `neutral`, `critical`의 요청 JSON을 넣는다. `raw_request`는 실제 `db.judgments.sent` 문자열 그대로다. 처음 다섯 해시 필드는 #7의 봉인 manifest와 일치해야 한다. 후보·목표·정책이 다른 trial은 같은 fixture로 합치지 않는다.

수집 전에 두 사람이 의미 보존 변형과 중요 조건 변경의 정답·필수 근거를 원문만 보고 검토한다. 검토가 끝나면 확인 fixture 내용의 SHA-256, 표본 수, 호출 상한을 `lock.json`에 고정한다. #7에 아직 원자료가 없는 과제나 `judgments.sent`가 없는 과제는 fixture를 만들지 않는다. 확인 원자료를 본 뒤 입력이나 기준을 바꾸지 않는다.
