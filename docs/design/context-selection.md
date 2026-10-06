# 맥락 고르기

| 항목 | 값 |
|---|---|
| 상태 | 제안 |
| 관련 결정 | [router가 후보 전체를 판단하고 코드 순위는 대체 순서로만 쓴다](../decisions/2026-10-01-router-all-candidates.md), [같은 뜻 찾기는 router에 맡기고 용어 카탈로그를 두지 않는다](../decisions/2026-10-01-router-decides-synonyms.md), [경쟁 구역은 기준값 없이 router 남김 확률 순으로 예산까지 채운다](../decisions/2026-10-02-fill-packet-by-probability.md) |

## 요약

맥락 고르기는 패킷, 결과 전달, 파일 순위에 넣을 항목을 고르는 기능이다. `core`의 `sessions`가 후보마다 파일 겹침, 단어 겹침, 최근성 순위를 매기고 RRF로 합친다. router는 후보 전체를 판단하고, 합친 순위는 router가 답하지 못했을 때의 순서와 같은 확률일 때의 순서로만 쓴다. 제약 식별과 대체는 [제약](constraints.md)에 있다.

## 동기

긴 채팅에는 도구 호출과 다른 에이전트 결과가 수백 개 쌓인다. 후보를 코드 순위의 상위 N개로 좁혀 router에 물으면 router가 남길 항목을 놓친다. 후보 전체를 판단하게 한 측정에서 router가 남긴 항목 중 RRF 상위 10개에 든 비율은 12.0%였고 무작위 기대값은 8.7%였다. 상위 40개도 44.5%였다([#116](https://github.com/woonyong-choi/saturn/issues/116) 측정). router가 답하지 못하면 대체 규칙이 생략이나 경로만이라 확실히 관련 있는 항목까지 빠진다. 이 기능은 router가 후보 전체를 판단하게 하고, 큰 요청은 질문 단위로 나눠 보내며, router가 실패하면 강제한 전환만 순위로 고른다.

## 예시

### 도구 호출 150개에서 패킷의 도구 결과 고르기

1. 사용자가 Claude로 40턴 작업한 뒤 Codex로 바꾸고 `로그인 실패 메시지 고쳐 줘`를 보낸다.
2. `sessions`는 도구 호출 150개에 파일 겹침, 단어 겹침, 최근성 순위를 매기고 RRF로 합친다.
3. 12턴의 `/v2/auth` 응답은 `auth/` 경로와 `로그인` 단어가 겹쳐 상위에 든다.
4. engine은 150개 전체를 `compact` 질문으로 router에 묻는다. 질문 300개와 state가 요청 한 건의 크기 한도를 넘으면 질문 단위로 나눠 보낸다.
5. `sessions`는 항목을 남김 확률이 높은 순으로, 같은 확률이면 순위 순으로 패킷의 경쟁 구역에 예산이 찰 때까지 채운다.

### router가 답하지 않을 때

1. 사용자가 모델을 고정해 새 session으로 옮기는 중에 패킷의 `compact` 판단이 응답하지 않는다.
2. engine은 5초 간격으로 두 번 다시 보내고, 10초 뒤에도 실패하면 로그에 `판단 모델 실패로 기록 선택을 건너뜁니다`를 남긴다.
3. `sessions`는 전환하고 RRF 순서대로 경쟁 구역의 예산까지 채운다.
4. `/v2/auth` 응답은 순위 상위라 router 판단 없이도 패킷에 들어간다.

## 상세 설계

### 보존 우선 선별 계약

[보존 우선 경로](context-management.md#보존-우선-경로)의 보호 본문은 후보 순위와 Jev 점수의 영향을 받지 않는다. 아래 기존 RRF·compact 요청·조회 구현을 재사용하며 새 검색 서비스나 별도 저장소를 추가하지 않는다.

- RRF와 Jev의 비교는 같은 보호 본문, 같은 도구 후보, 같은 전문 예산, 같은 원문 조회 권한을 사용한다. 선택한 ID와 전문·발췌·생략만 달라진다.
- Jev의 질문에는 도구 결과가 미래 작업에 필요한지 추측하게 하기보다 현재 목표·유효한 지시·사용 가능한 근거와의 관련성을 묻는다. 판단 state에는 보존된 대화의 범위와 생략량을 남긴다. 입력 한도 때문에 필수 목표·정정을 싣지 못하면 판단 미적용으로 기록한다.
- RRF 순위는 판단 실패의 고정 대체다. 대체 동작은 요청한 selector와 실제 적용 selector를 분리해 기록한다. 실패한 판단이 보호 본문을 줄일 수 없다.
- 도구 결과 중간에 있는 근거도 평가한다. 앞 4,000자나 앞·뒤 발췌만 보인 요청은 그 관측 범위를 기록하고 원문 전체를 판정했다고 표시하지 않는다.
- 현재 검색용 `evidence::select`의 확률 기준과 패킷용 확률 순 정렬은 서로 다른 계약이다. 최소 패킷 실험은 기존 패킷 정렬을 사용한다. 검색 기준을 패킷에 복사하거나 결과를 보고 기준값을 바꾸지 않는다.
- 사용자 원문 조회는 `InputId`와 `LedgerSeq`를 혼용하지 않는 종류 있는 참조로 확장한다. 기존 숫자 도구 조회는 호환한다. 현재 권한으로 필터링한 뒤 후보 집합을 만들고 읽을 때 다시 검사한다.
- 답변 전 검색은 우선 새 session의 첫 작업 입력에만 적용한다. engine 내부에서 검색하고 적용 직전 revision을 비교한다. 모든 턴에 검색·LLM 추출을 강제하는 확장은 별도 이득 확인 뒤 결정한다.

#### 관련 원문 선주입

실험 설정 `context.select.related`가 `rank`나 `jev`일 때만 새 session을 여는 입력의 패킷에 적용한다([설정](settings.md)). 기본 `off`는 아무 것도 바꾸지 않는다. 보관 session으로 돌아가는 변경분과 맥락 정리의 새 session에는 적용하지 않는다. 구현은 `engine/src/related.rs`와 `evidence.rs`의 `related_search`, `related_recheck`, 패킷 조립은 core `PacketSource::related`다.

1. 후보 집합, 읽기 범위, 비밀 가림, 순위는 근거 검색과 같다. 검색어는 접수한 입력 본문이다. 사용자 입력, 끼워 넣은 입력, 에이전트 글, 도구 기록이 모두 후보다.
2. 패킷이 이미 싣는 대화 본문은 종류 있는 번호(`input`·`text`·`steer`와 번호)로 걸러낸다. 글자가 같다는 이유로 걸러내지 않으므로 반복한 입력은 둘 다 남는다. 같은 도구 기록이 관련 원문으로 전문이 들어가면 경쟁 구역에서는 같은 기록 번호를 뺀다.
3. 순위 위 20건까지 차례로 `P_max`와 전송 상한 `P_send` 중 작은 값에서 고정 구역을 뺀 남은 글자 수에 맞는지 본다. 맞으면 원문 전체를 넣고, 맞지 않으면 줄여 넣지 않고 그 기록을 `budget`으로 뺀다. 경쟁 구역은 관련 원문을 넣은 뒤 남은 예산으로 채운다.
4. 넣은 원문은 패킷 고정 구역의 `Related original records` 구역에 `[종류:채팅:번호:해시]` 한 줄 뒤 원문으로 실린다. 이 참조는 `saturn evidence read`가 읽는 참조와 같고 고정 구역이라 기존 전송 직전 검사(해시 일치, 앞에서부터 글자 그대로)를 그대로 받는다.
5. 보내기 직전(`open_with_plan`)에 채팅 revision(기록의 마지막 번호)과 입력 본문, 고른 기록과 생략 후보마다 해시와 현재 읽기 권한을 다시 맞춘다. revision이나 입력이 달라졌으면 늦은 결과라 모두 버리고(`stale_revision`), 원문이 바뀌었으면 `changed`로 그 기록만 뺀 패킷을 다시 만든다. 읽기 범위 밖이 됐거나 기록에서 사라진 참조는 근거 행에서도 제거한다. 뺀 자리에 오래된 내용이나 다른 내용을 넣지 않는다.
6. 후보가 하나도 없으면 `no_candidates`, 검색이나 재검사를 하지 못하면 `retrieval_failed`로 분류하고 입력은 그대로 보낸다. 패킷 자체가 없는 첫 입력에는 항목 행을 만들 수 없어 이 분류를 저장하지 않으며 검색 실패만 진단 로그에 남는다. 맥락 한도로 거절돼 패킷을 줄일 때는 관련 원문을 줄이지 않고 모두 `reduced`로 뺀다.
7. 근거는 기존 전달 패킷 기록의 항목 행이다. 구역 `Related-<종류>`, 번호, 원문 해시(`body_hash`), 선택 방식 `related_rank`나 `related_jev`, 들어간 모양(`Full`)이나 빠진 이유를 남기고, 기록 하나에 속하지 않는 이유는 구역 `Related`에 후보를 읽은 revision을 번호로 둔 한 행이다. 패킷 행의 입력 번호와 `chat_revision`이 검색한 입력과 기록 revision이다. 별도 저장소나 검색 서비스는 없다. 읽기 권한이 없는 기록은 후보가 아니라 있다는 사실도 남기지 않는다.

#### 관련 원문 Jev 실험 계약

`context.select.related = jev`는 위 `rank`와 같은 입력·채팅의 `related_search` 결과에서 보호 본문에 이미 든 종류 있는 번호를 뺀 상위 20건만 본다. 두 방식의 후보 순서, 현재 읽기 권한, 원문 해시, 전문 예산은 같다. 선택 방법만 다르며 기본값은 계속 `off`다.

1. 후보 스냅샷에는 `(kind, chat, id, hash)`와 순위, 원문, 검색한 기록 마지막 번호를 둔다. 한 요청 안에서 후보 순서대로 1부터 임시 ordinal을 부여하고 답을 이 스냅샷으로만 되돌린다. ordinal은 저장 참조가 아니며 패킷과 근거에는 종류 있는 참조와 해시만 남긴다. 도구 호출·결과 번호용 `compact` 질문을 재사용하지 않는다.
2. 독립된 `related@2.0` 질문 집합에서 후보마다 `candidate_<ordinal>_direct`의 `noul` 답 하나를 묻는다. 질문은 "이 관측 발췌가 접수한 사용자 요청에 답하는 데 필요한 정보를 직접 담고 있는가"이고 발췌가 원문 전체가 아님을 밝힌다. state에는 접수한 입력과 현재 작업 범위를, 질문에는 종류 있는 참조와 가린 원문 발췌의 관측·생략 길이를 싣는다. 모든 후보의 응답 확률이 유한한 0~1 값이고 요청한 답이 모두 `noul`로 있을 때만 판단을 쓴다. 접수 설정의 `router.thresholds.min_confidence`는 후보별로 확신 있는 긍정(`P(yes) > 0.5`이고 확신이 그 값 이상, 기본 0.6이면 `P(yes) >= 0.8`)을 가리는 데만 쓴다. 확신 있는 긍정을 확률 내림차순, 동률은 기존 순위 순으로 앞에 놓고 나머지 후보를 기존 순위 순으로 모두 뒤에 붙인 뒤 동일한 전문 예산에 채운다. 부정이나 불확실한 답으로 후보를 빼지 않는다. 확신 있는 긍정이 없거나 만든 순서가 `rank`와 같으면 `rank`를 적용하고 `jev_no_promotion`을 남긴다. 그래서 Jev가 아무것도 바꾸지 않았는데 적용했다고 기록하지 않는다. 이 정책은 #541 B2의 사전 시험용이며 품질·비용 이점을 주장하지 않는다.
3. 요청은 별도 작업으로 보내 engine을 막지 않는다. 답을 적용하기 직전에 채팅 번호, 접수 입력 번호와 본문, 접수 설정 번호와 현재 `related` 값, 검색한 기록 마지막 번호, 후보별 종류 있는 참조와 해시, 새 session 대상이 모두 같아야 한다. 하나라도 다르면 늦은 판단으로 버리고 현재 자료로 `rank`를 다시 만든다. 기한 뒤 답, router 실패, 답 없음, 잘못된 확률, 확신 있는 긍정 없음, 요청 한도 초과도 `rank`로 고정한다. rank 검색 자체가 실패하거나 후보가 없으면 선주입하지 않는다. 적용 직전 원문별 현재 권한·해시 재검사는 선택 방식과 관계없이 그대로 한다.
4. 요청한 방식과 실제 적용 방식, 판단 실패·늦음·대체 이유를 기존 판단 기록과 전달 패킷 항목 행에 남긴다. Jev 결과와 rank 결과 모두 같은 전문 예산과 같은 생략 사유를 쓴다. 품질·비용은 [#541](https://github.com/woonyong-choi/saturn/issues/541)의 B2에서 별도로 측정한다.

구현은 `related` 질문 집합(`saturn-terminal/core/src/routers`), 후보 스냅샷과 적용 직전 비교(`saturn-terminal/engine/src/related.rs`), 기존 `compact` 대기·재개 경로(`packet_select.rs`)의 답 자리 일반화(`Trigger::Related`)로 이 계약을 따른다. 달라진 점은 다음과 같다.

- 기록에 남는 이유는 `jev_late`, `jev_failed`, `jev_no_promotion`, `jev_invalid`, `jev_stale`, `jev_too_large`다. 후보를 빼는 이유는 없다. `rank`로 고정하면 기록 행의 선택 방식은 `related_rank`이고 요청한 `related_jev`는 구역 `Related`의 이유 행에 남는다.
- 후보 원문은 질문에 앞 120자와 뒤 220자 발췌로만 싣는다(초안). 발췌는 유니코드 글자 경계로 자르고 질문이 `first 120 and last 220 of N characters, M omitted`로 관측·생략 길이를 밝히며 `clipped`로 표시한다. 340자 이하 원문은 전문을 한 번만 싣고 `full record, N characters`로 적는다. 발췌만 본 판단은 판단 기록에 `clipped`를 더한다. `state`와 질문 하나가 한도를 넘는 경우는 호출 없이 `rank`로 간다. 로컬 router는 분할 전송을 지원하지 않아 전체 질문이 한 조각을 넘으면 `jev_too_large`로 대체한다.
- 판단 기록은 질문 집합 `related@2.0`이고, 기록 번호용 `compact` 질문과 답 자리를 나눠 쓴다. 패킷 `compact`와 함께 켜면 각자 한 번씩 묻고 한 번씩 기록한다.

보존 우선 선별은 구현 전 계약이고, 관련 원문 선주입은 위 실험 설정 뒤에서만 구현했다. 아래의 도구 전용 원문 조회와 기존 패킷 호출은 이미 있는 경로다. 전체 범위와 독립 검증 순서는 [비교 보고서](../notes/references/context-preservation.md#전체-범위와-최소-범위)를 따른다.

### 고르기를 쓰는 곳

| 쓰는 곳 | 후보 | router 질문 |
|---|---|---|
| 패킷의 경쟁 구역 | 이 채팅의 도구 호출과 결과 | `compact` |
| 기록 번호로 결과 전달 | 기록 번호 뒤 다른 에이전트의 결과 요약 | `compact` |
| 파일 순위 | 코드 검색이 찾은 파일 | `file-rank` |

- 문서 조각을 고르는 `context-select`는 이 흐름을 쓰지 않는다. 문서 조각은 `doc-filter`의 인젝션 판단을 거쳐야 하므로 router 없이 순위만으로 넣지 않기 위해서다.
- `Reasoning` 종류의 도구 호출은 기록에 남기되 후보에서 뺀다. 추론은 도구 결과가 아니라 후보 수와 메모를 provider마다 다르게 만들기 때문이다.
- 패킷의 구역과 채우기 규칙은 [맥락 정리](context-management.md)에, 결과 전달은 [provider 연결과 session](providers-and-sessions.md)에 있다.

### 순위 채널

| 채널 | 쓰는 값 | 순위 |
|---|---|---|
| 파일 겹침 | 후보가 건드린 파일 경로. 도구 호출 이벤트의 경로 목록(`detail.paths`)에서 꺼낸다. | 기준 파일과 겹치는 경로 수가 많을수록 위 |
| 단어 겹침 | 후보의 글과 마지막 입력을 단어 조각으로 나눈 것 | BM25 점수가 높을수록 위 |
| 최근성 | 후보의 기록 번호 | 클수록 위 |

- 기준 파일은 마지막 입력에 나온 경로와 메인 session이 최근 3턴에 건드린 파일이다.
- 경로는 NFC로 정규화하고 앞의 `./`를 떼고 비교한다. 같은 파일이 표기 차이로 어긋나지 않게 하기 위해서다.
- 단어 겹침의 BM25는 `k1 = 1.2`, `b = 0.75`(초안)이고 마지막 입력의 조각은 중복 없이 한 번씩 센다.
- 한 채널 안에서 값이 같은 후보는 같은 순위다. 값이 같은데 순서만으로 점수가 갈리지 않게 하기 위해서다.
- 채널에 값이 없는 후보는 그 채널 순위에서 빠진다. 최근성은 모든 후보에 있다.
- 세 채널 모두 기록에 이미 있는 값을 계산만 한다. 라벨링이나 모델 호출이 없어 `core`가 파일, 네트워크, 프로세스를 다루지 않고 계산하기 위해서다.
- 셸 명령처럼 경로가 인자에 따로 없는 도구 호출은 경로 모양의 글자만 꺼낸다(초안).

### 단어 조각

1. 글을 유니코드 NFC로 정규화한다. macOS 파일 이름처럼 자모로 풀린 한글을 음절로 합쳐야 한글 구간으로 읽기 때문이다.
2. 글자마다 유니코드 범위로 종류를 정하고, 종류가 바뀌는 곳에서 나눈다. `로그인login`은 `로그인`과 `login`이 된다.
3. 종류마다 다음 표대로 조각을 만든다.

| 종류 | 예 | 조각 |
|---|---|---|
| 한글 | `로그인실패` | 글자 2개씩 겹치게: `로그`, `그인`, `인실`, `실패` |
| 라틴 문자(악센트 포함) | `authLogin.rs` | 소문자로 바꾸고 camelCase, snake_case, kebab-case 경계에서 나눈 단어: `auth`, `login`, `rs` |
| 숫자 | `404`, `v2` | 이어진 숫자 하나 |
| 한자, 가나 | `設定`, `ログイン` | 한글과 같이 글자 2개씩 |
| 기호 | `/`, `.`, `_`, `-`, `::` | 조각을 만들지 않고 나누는 경계로만 쓴다 |
| 공백, 이모지, 제어 문자 | | 버린다 |

- 한글을 글자 2개 단위로 자르는 것은 띄어쓰기 누락, 조사, 복합명사에서도 겹침을 찾기 위해서다. 한국어 검색에서 n-gram 색인은 사전 없이 복합명사를 다루고 형태소 기반 색인보다 효과가 좋았다(Lee & Ahn, SIGIR 1996; Lee, Cho, Park, Information Processing & Management 1999).
- 한자와 가나를 글자 2개 단위로 자르는 것은 중국어와 일본어도 띄어쓰기가 단어 경계가 아니기 때문이다. 버리면 그 언어 사용자는 단어 겹침 채널을 쓰지 못한다.
- 기호를 경계로만 쓰는 것은 `auth/login.rs`의 `/`와 `.`처럼 식별자 경계 역할을 하지만 그 자체로는 뜻이 없기 때문이다.
- 영문을 식별자 경계에서 나누는 것은 기록의 영문 대부분이 코드 식별자와 경로이기 때문이다.
- 한자와 가나는 한 종류로 본다. 일본어는 한자와 가나를 섞어 한 단어를 쓰기 때문이다.
- 한 글자뿐인 한글, 한자, 가나 구간은 그 글자 하나를 조각으로 둔다. 한 글자 단어를 버리지 않기 위해서다.
- 라틴 밖 알파벳 문자(키릴, 그리스 문자 등)는 라틴 문자와 같은 규칙으로 자르되 다른 종류로 본다.
- 오타는 오타 글자가 든 조각만 빠지므로 점수가 낮아질 뿐 0이 되지 않는다.
- 같은 뜻의 다른 말과 번역어는 router가 후보 전체를 판단하면서 직접 판단한다. 용어 카탈로그와 임베딩 채널은 두지 않는다([결정 기록](../decisions/2026-10-01-router-decides-synonyms.md)). 작은 다국어 모델 두 개로 잰 결과, 한국어 설명 질의의 정답 코드 묶음을 상위 10개에 올린 비율은 단어 기반과 같은 0.0%였고, 단어가 겹치는 질의의 상위 10개 재현율은 10.5%p 이상 낮아졌다([실험 보고서](../experiments/embedding-synonym/report.md)). 설치 크기 342.5MB 이상과 상주 메모리 287.4MB 이상도 든다.
- 한글을 자모 3개 단위로 자르지 않는다. 오타 질의 재현율 이득이 0.9%p [−0.8, 2.6]에 그치고 오타 없는 질의의 1위 정밀도가 10.0%p 떨어졌기 때문이다([실험 결과](../experiments/wordpiece-typo-recall/report.md)).
- 영문 식별자 단어를 글자 4개 단위로 바꾸지 않는다. 오타 질의 재현율은 11.6%p 올랐지만 오타 없는 질의의 1위 정밀도가 5.2%p 떨어졌기 때문이다([실험 결과](../experiments/wordpiece-typo-recall/report.md)). 영문 글자 n-gram의 근거는 McNamee & Mayfield, Information Retrieval 2004다.

### 순위 합치기

`sessions`는 채널 순위를 RRF(Reciprocal Rank Fusion)로 합친다. `r_c`는 채널 c에서 후보의 순위(1부터)이고, `k`는 합치기 상수다.

```text
점수 = Σ_c 1 / (k + r_c)
```

- 점수 대신 순위를 쓴다. 채널마다 값의 단위가 달라 그대로 더할 수 없기 때문이다.
- `k`의 기본값은 60이고 설정 `context.select.rrf_k`로 바꾼다. 실측에서 `k` 10, 30, 60, 100의 상위 40개 기준 차이는 1.0%p 이하였다([실험 결과](../experiments/rrf-k-top-n/report.md)).
- 원 논문은 여러 검색 결과를 합칠 때 `k`=60을 썼다(Cormack, Clarke, Büttcher, SIGIR 2009). `k`가 클수록 한 채널의 1등보다 여러 채널에 고르게 든 후보가 이긴다.
- 점수가 같으면 기록 번호가 큰 후보를 위에 둔다.
- `k`는 router가 답하지 못했을 때의 순서와 같은 확률일 때의 순서에만 쓰인다.

### router에 넘기기

`core`의 요청 만들기와 답 합치기를 engine이 패킷을 만들 때 부르는 연결은 실험 옵션 `context.select.packet = jev`일 때만 켜진다([#380](https://github.com/woonyong-choi/saturn/issues/380)). 기본(`rrf`)은 RRF 순서로만 채운다.

1. 후보 전체를 router에 묻는다. 후보 수로 줄이지 않는다.
2. 요청이 크기 한도를 넘으면 질문 단위로 나눠 여러 요청으로 병렬 전송하고, 조각마다 같은 state를 싣는다. 동시 수와 한도는 [router 호출](router.md#router-호출)에 있다.
3. 항목의 남김 확률은 `call_<id>_keep`과 `result_<id>_keep` 중 큰 값이다. 하나만 답했으면 그 값이다.
4. 최종 순서는 답이 있는 항목을 남김 확률이 높은 순으로 두고, 같은 확률이면 RRF 순으로 둔다. 기준값은 없고 확률이 낮은 항목도 빼지 않는다.
5. router가 답하지 못한 항목은 답이 있는 항목 뒤에 RRF 순으로 둔다. 실패한 조각의 항목도 같다.
6. router가 전부 답하지 못하면 재시도가 끝난 뒤 판단 없이 진행한다. router가 시작한 전환은 건너뛰고 현재 모델로 진행하며, 사용자가 고정했거나 맥락 크기 규칙이 시작한 전환은 RRF 순서로 경쟁 구역의 예산까지 채운다. 재시도와 로그는 [router 실패](router.md#router-실패)에 있다.

- router가 남길 항목을 순위로 미리 자르지 않기 위해서다. 후보 전체를 판단한 측정에서 남은 항목 중 RRF 상위 10개에 든 비율은 12.0% [9.2, 15.6]였고 무작위 기대값은 8.7%였다. 상위 40개도 44.5% [39.7, 49.4]였고, 95%에 닿으려면 후보 중앙값 127개보다 많은 132~133개가 필요했다([#116](https://github.com/woonyong-choi/saturn/issues/116) 측정).
- 전체를 묻는 비용은 낮다. 기준 router의 입력 비용은 100만 토큰당 $0.042이고 같은 질문의 일치율은 98.4%였다([#116](https://github.com/woonyong-choi/saturn/issues/116) 측정).
- 기준값을 두지 않는다. 근거 항목의 남김 확률은 평균 0.372, 최댓값 0.65여서 기준값 0.5가 근거 항목 624개 중 549개(88.0%)를 버렸고, 기준값 없이 확률 순으로 채우면 필요한 근거가 모두 든 질문이 11.7%에서 65.6%가 됐다. 확률은 근거와 비근거를 잘 가르므로(AUC 0.940) 순서에만 쓴다([결정 기록](../decisions/2026-10-02-fill-packet-by-probability.md), [후속 분석](../experiments/handoff-packet-quality/report.md#후속-분석-원인)).
- RRF 순위는 router 판단을 보조하는 값이다. router가 답하지 못할 때의 순서와 같은 확률일 때의 순서만 정하므로, 순위가 낮아도 router가 답하면 먼저 들어간다.
- 나누는 규칙은 `core`가 요청 목록을 만드는 순수 함수이고, 전송과 응답 모으기는 engine이 한다. `core`가 네트워크를 다루지 않기 위해서다.

### compact 요청의 state와 질문

`compact` 요청은 후보의 내용을 질문에, 지금 하려는 일을 `state`에 싣는다.

1. `state`는 마지막 사용자 입력 원문과 그 앞 사용자 입력 3개 원문이다. 입력 하나는 앞 2,000자까지 싣는다(초안).
2. `call_<기록 번호>_keep` 질문에는 `<도구 이름> <인자 JSON>`을, `result_<기록 번호>_keep` 질문에는 같은 호출과 결과 앞 4,000자를 싣는다.
3. 기록 번호로 결과를 전달할 때 후보인 다른 에이전트의 결과 요약은 결과 자리에 요약문을 싣는다(초안).
4. `state`와 질문 모두 [router 호출](router.md#router-호출)의 규칙대로 비밀값을 가리고 절대 경로를 `[abs]/이름`으로 바꾼다.

- 후보의 내용을 `state`가 아니라 질문에 싣는 것은 요청을 질문 단위로 나눌 때 조각마다 같은 `state`를 되풀이해도 그 크기가 후보 수에 따라 늘지 않게 하기 위해서다.
- 제약 목록은 `state`에 넣지 않는다. 제약은 패킷의 고정 구역에 전부 들어가므로 후보를 남길지 판단하는 데 쓰지 않는다.
- 이 형식(결과 앞 4,000자, 마지막 입력과 앞 입력 3개)으로 후보 전체를 판단한 패킷은 판단 없는 순위 패킷보다 정답률이 50.7%p [44.7, 56.7] 높았다([새 패킷 규칙의 전환 품질 재측정](../experiments/handoff-packet-quality-v2/report.md)).

### 도구 결과 메모

축약본은 결과 앞 300자와 한 줄 메모다. 메모는 도구 종류별 틀로 `sessions`가 만든다. 모델 호출 없이 같은 결과에서 늘 같은 메모를 만들기 위해서다.

| 도구 종류 | 메모 틀 |
|---|---|
| 셸 명령 | 명령 · 종료 코드 · 줄 수 · `error`, `failed`, `panic`이 든 첫 줄 |
| 테스트 실행 | 통과 수, 실패 수, 실패한 테스트 이름 |
| 파일 읽기 | 경로 · 읽은 줄 범위 |
| 파일 수정 | 경로 · 더한 줄 수와 지운 줄 수 |
| 웹 요청 | 주소 · 상태 코드 |
| 그 밖 | 메모 없음 |

engine의 `providers`가 provider 도구 이름을 Saturn 도구 종류로 바꾸고, 경로, 읽은 줄 범위, 바뀐 줄 수, 종료 코드를 이벤트에 싣는다. provider 고유 이름을 `providers/codex`, `providers/claude` 안에만 두기 위해서다. 이벤트 필드는 [provider 연결과 session](providers-and-sessions.md#이벤트-수신과-변환)에 있다.

### 근거 검색과 원문 조회

패킷이 줄이거나 뺀 기록을 에이전트가 번호로 다시 읽는 길이다. 순위, Jev, 작업 LLM이 같은 후보에서 고르고 다시 읽는 비교 실험([#540](https://github.com/woonyong-choi/saturn/issues/540))도 이 길을 쓴다. 새 저장소와 새 색인은 없다. 후보와 원문은 기록 저장소의 도구 호출과 결과, 저장된 사용자 입력, 메인 에이전트가 보인 글이고 순위는 위의 어휘·파일·최근성 RRF를 그대로 쓴다.

후보는 종류(`kind`)가 있는 원문 한 건이다. 번호는 종류마다 따로 세므로 같은 숫자가 종류가 다르면 다른 원문이다. 실행을 연 입력의 번호는 그 실행의 첫 기록 번호라 첫 기록이 도구 호출이나 글이면 그 번호와 겹친다.

| 종류 | 원문 | 번호 |
|---|---|---|
| `tool` | 도구 호출과 결과. 패킷의 경쟁 구역과 같은 원문 | 채팅의 기록 번호. 패킷 항목 앞의 `#41` |
| `input` | 실행을 연 사용자 입력 | 그 실행의 첫 기록 번호 |
| `steer` | 실행 중에 끼워 넣어 적용한 사용자 입력 | 입력 번호 |
| `text` | 메인 에이전트가 보인 글 | 그 글의 기록 번호 |

- 후보가 아닌 것: `PacketReply`, 하위 에이전트 글, 숨겨진 추론, 빈 글, router 키나 출입증 모양이 든 글(출력 마스킹 규칙으로 가려지는 글). 마지막은 있다는 사실도 알리지 않으므로 읽으려 하면 `NotFound`다.
- `core`의 `sessions::evidence`가 후보 집합 `CandidateSet`을 만든다. 집합은 종류마다 하나이고, 검색 응답의 집합 해시는 후보가 있는 종류의 집합 해시를 종류 이름과 함께 이은 값의 SHA-256이다. 순위는 종류를 섞은 한 목록에 매기며 시간순 자리를 번호로 삼는다(점수가 같으면 나중 것이 위).

| 후보 값 | 뜻 |
|---|---|
| `kind`, `chat`, `id` | 원문 종류, 채팅, 종류별 번호. 도구는 패킷 항목 앞의 `#41`과 같다. |
| `project` | 기록을 만든 채팅의 작업 폴더 |
| `at_ms` | 기록 시각 |
| `chars` | 원문 글자 수. 원문 범위는 `0..chars`다. |
| `excerpt` | 원문 앞 300글자 |
| `excerpt_start`, `excerpt_end` | 발췌의 원문 글자 범위. 현재는 `0..excerpt_end` |
| `hash` | 원문의 SHA-256 |

- 집합은 번호 순으로 보관한다. 입력 순서가 바뀌어도 같은 집합이고 같은 집합 해시다. 같은 번호가 둘이면 만들지 않는다.
- 원문을 읽을 때는 번호가 집합에 있는지, 같은 폴더의 기록인지, 후보를 볼 때의 해시와 같은지 차례로 본다. 없는 번호, 다른 폴더, 바뀐 원문은 각각 다른 오류다.
- 도구 호출에 결과가 아직 없다가 나중에 오면 원문이 바뀌므로 해시가 달라진다. 그때는 후보를 다시 본다.

선택 방법은 `select`가 바꿔 끼운다. 같은 집합, 같은 순위, 같은 전문 예산(원문 글자 수)에서 제안만 다르다.

| 방법 | 제안 | 고르는 규칙 |
|---|---|---|
| 순위 | 없음 | RRF 순위 그대로 예산이 찰 때까지 채운다. 예산에 안 드는 후보는 건너뛰고 작은 후보를 계속 본다. |
| Jev | 후보마다 남김 확률 | 확률이 0.5 이상인 후보만 확률이 높은 순으로, 같은 확률이면 순위 순으로 채운다. 0.5는 첫 실험을 위한 가정이다. |
| 작업 LLM | 고른 번호 | 고른 순서로 채운다. |

- Jev와 작업 LLM이 판정하지 못하면(호출 오류, 확률이 0.5 이상인 후보가 없음, 없는 번호, 겹친 번호, 0과 1을 벗어난 확률, 빈 선택) 순위 선택으로 돌아가고 사유를 `undecided`로 남긴다. 추가 요약 LLM은 부르지 않는다. 실제 적용한 방법(`applied`)과 요청한 방법(`requested`)이 따로 남아 대체를 적용으로 세지 않는다.
- 검색용 `Proposal`은 선택 결과를 받는 계약이다. 패킷용 Jev 호출은 `engine/src/packet_select.rs`의 비동기 경로에 연결되어 있다. 작업 LLM 검색 선별의 제품 호출 연결은 없다. [#380](https://github.com/woonyong-choi/saturn/issues/380)은 보호 본문과 도구 후보를 분리하는 남은 변경을 맡고, 조건 비교 수집은 [#540](https://github.com/woonyong-choi/saturn/issues/540)이 맡는다.

에이전트는 작업 안에서 `saturn evidence`로 기록을 찾고 읽는다. 지원하는 provider는 둘 다 셸 명령을 실행하므로 공통 길은 하나다.

| 명령 | 하는 일 |
|---|---|
| `saturn evidence search <검색어>... [--limit N]` | 검색어는 따옴표 없이 여러 단어로 써도 공백 하나로 이어 한 검색어로 본다. 후보를 순위 순으로 보인다. 첫 줄은 집합 해시와 후보 수, 이어서 한 줄에 한 후보다. 상한은 50개다. |
| `saturn evidence read <번호> [--hash H] [--offset N] [--limit N]` | 옛 형식. 도구 호출 기록 번호로 읽는다. 한 번에 최대 20000글자이고 더 있으면 첫 줄에 `next_offset`이 있다. |
| `saturn evidence read <종류>:<채팅>:<번호>:<해시> [--offset N] [--limit N]` | 종류별 참조로 읽는다. 해시는 SHA-256 16진수 소문자 64자이고 `--hash`를 함께 주면 참조 안의 해시와 같아야 한다. |

- 검색 결과 한 줄은 도구 호출이면 숫자 첫 열을 유지한다: `#번호`, `at_ms`, `chars`, `hash`, `excerpt 0..끝`, 발췌를 탭으로 잇는다. 입력과 글은 첫 열이 읽을 때 그대로 넣는 참조 `<종류>:<채팅>:<번호>:<해시>`이고 이어서 `at_ms`, `chars`, 발췌 범위와 발췌다.
- 참조로 읽은 원문의 첫 줄은 `<종류>:<채팅>:<번호>:<해시> chars N offset N`이고 더 있으면 `next_offset N`이 붙는다. 옛 번호로 읽으면 옛 첫 줄(`#번호 hash H chars N offset N`)이다.
- 프로토콜은 `EvidenceRead`에 `kind`와 `chat`(둘 다 없어도 되는 값)을 더했고 후보와 읽은 원문의 응답에 `kind`와 `chat`을 더했다. `kind`가 없는 요청은 옛 형식이라 도구 호출 번호로 읽고 `chat`은 보지 않는다. 새 CLI가 옛 engine에 종류별 참조를 보내면 옛 engine이 종류를 무시할 수 있으므로, CLI는 응답의 종류·채팅·번호·해시가 요청과 다르면 본문을 출력하지 않고 거절한다. 판 번호는 올리지 않는다.

- 출입증(`SATURN_PASS`)으로 접속하고 출입증을 준 채팅의 기록만 본다. 출입증이 없거나 회수됐으면 거절한다. 에이전트 작업 밖에서는 쓸 수 없다.
- 옛 형식의 기록 번호는 채팅마다 센다. 다른 채팅의 번호를 넣어도 그 채팅의 기록이 아니라 이 채팅의 같은 도구 호출 번호를 본다. 종류별 참조는 채팅을 함께 적고, 출입증을 준 채팅과 다르거나 채팅이 없으면 `NotFound`로 거절한다.
- 종류별 참조는 해시가 필수이고 읽을 때마다 출입증, 채팅 범위, 현재 읽기 권한, 해시를 다시 본다. 검색 때의 결과를 믿고 읽지 않는다.
- 읽기 범위는 작업 폴더와 더한 폴더다. 도구 호출이 건드린 경로가 범위 밖이거나 `permission.read`의 `deny`와 일치하면 그 기록은 후보에서 빠지고 번호로 읽으려 해도 거절한다. 권한이 나중에 좁아져도 지난 기록으로 우회하지 못하게 하기 위해서다. 읽을 때마다 채팅의 가장 최근 설정으로 다시 나눈다. 입력과 글은 파일을 가리키지 않으므로 이 파일 규칙의 대상이 아니고, 출입증과 채팅 범위, 비밀 가리기만 받는다.
- 조회는 기록 저장소를 읽을 뿐 파일을 읽지 않는다. 입력은 기록 저장소에 접수되고 실행에 쓰인 뒤의 것만 후보이므로, 조회가 입력을 provider로 보내는 순서나 인계 패킷의 기록 번호(revision)를 바꾸지 않는다. 조회마다 종류, 번호, 결과(`Ok`, `NotFound`, `Stale`, `Scope`, `Unreachable`), 돌려준 양을 `evidence_lookups`에 남긴다([기록](records.md#근거-조회-기록)). 조회한 기록이 없는데 0회로 보이는 일이 없게 하기 위해서다. 이 표의 종류는 검색과 읽기이고 원문 종류(`kind`)는 남기지 않는다.
- 에이전트가 이 명령을 쓰게 알리는 것은 설정 `context.evidence.lookup`(기본 거짓)이다. 켜면 경쟁 구역에서 원문 아닌 모양으로 들어가거나 빠진 기록이 있는 패킷의 끝에 `saturn evidence read <number>` 안내 한 줄이 붙는다. 안내는 경쟁 구역 예산에 든다. 꺼 두면 패킷은 지금과 같다.
- 셸 명령이라 권한은 Saturn 규칙을 그대로 받는다. 새 스킬이나 MCP 서버를 설치하지 않는다. Claude는 Bash 샌드박스가 Unix 소켓 접속을 막으므로 실행별 `--settings`의 `sandbox.network.allowUnixSockets`에 engine 소켓 경로를 넣는다(`providers/claude`). 경로가 심볼릭 링크를 지나면 같은 소켓의 실제 경로도 함께 넣고 다른 소켓은 허용하지 않는다. Codex는 허가한 명령을 작업 폴더 쓰기 샌드박스 안에서 돌리고, 그 샌드박스는 셸이 직접 실행하는 마지막 명령에만 소켓 접속을 허용한다. 그래서 `saturn evidence read 13`이나 `true && saturn evidence read 13`은 닿고, 파이프나 `;`로 이어 붙여 하위 프로세스로 도는 `saturn evidence read 13 | tail -n 3`은 `Operation not permitted`로 닿지 못한다(Codex 0.158.0, 실제 Codex로 확인). 소켓 허용을 Codex 설정으로 넓히지 않는다. 닿지 못하면 `saturn evidence`가 첫머리에 `Error: saturn evidence unreachable (read 13)`와 함께 한 명령으로 실행하고 `--limit`과 `--offset`을 쓰라는 안내를 낸다. 요청이 engine에 오지 않으므로 engine이 도구 결과의 이 첫머리를 읽어 그 시도를 `Unreachable`로 센다. 결과 첫머리가 아니거나 번호가 틀린 같은 글은 세지 않는다. 오류를 `2>&1 | tail`로 잘라 첫머리가 없어지면 그 시도는 세지 못한다.

### 오류 처리

| 상황 | 동작 |
|---|---|
| `compact`, `file-rank` router 호출이 재시도 뒤에도 실패 | router가 시작한 전환은 건너뛰고 현재 모델로 진행한다. 그 밖에는 RRF 순서로 경쟁 구역의 예산까지 채운다. |
| 요청 조각 일부 실패 | 실패한 조각의 항목은 남기고, 답이 있는 남긴 항목 뒤에 RRF 순으로 둔다. |
| 크기 한도를 넘는 `state` | 질문 하나도 담을 수 없으므로 요청을 만들지 않고 RRF 순서로 채운다. |
| 도구 호출 인자에 경로 없음 | 파일 겹침 채널에서 그 후보를 뺀다. |
| 근거 조회의 출입증이 없거나 회수됨 | 거절한다(`pass is unknown or revoked`). |
| 명령이 샌드박스 때문에 engine에 닿지 못함 | `Unreachable`로 센다. 오류에 표지와 한 명령으로 실행하라는 안내를 싣는다. |
| 없는 기록 번호, 다른 채팅의 참조, 채팅이 없는 참조, 비밀이 든 글 | `NotFound`로 거절한다. |
| 후보를 본 뒤 원문이 바뀜(해시 불일치), 종류별 참조에 해시가 없음 | `Stale`로 거절한다. |
| 읽기 범위 밖 파일이거나 읽기 규칙이 거부하는 경로의 도구 호출 기록 | `Scope`로 거절하고 후보에서도 뺀다. |

### 요구사항

| 요구사항 | 검증 계획 |
|---|---|
| 도구 결과 메모의 종료 코드, 경로, 줄 수를 provider와 무관하게 이벤트에서 얻는다. | [provider 연결과 session](providers-and-sessions.md#요구사항)의 도구 호출 값 행 |
| 후보 전체를 router에 묻는다. | `saturn-terminal/core/src/routers/tests.rs`의 `compact_questions_150_candidates_ask_all` |
| 요청이 크기 한도를 넘으면 질문 단위로 나누고 조각마다 같은 state를 싣는다. | `saturn-terminal/core/src/routers/split.rs`의 `split_request_over_limit_splits_by_question_with_same_state`, `saturn-terminal/core/src/routers/tests.rs`의 `compact_requests_large_state_splits_and_every_piece_carries_state` |
| 최종 순서는 답이 있는 항목의 남김 확률 순이고 같은 확률이면 RRF 순이며 확률이 낮은 항목도 빼지 않는다. | `saturn-terminal/core/src/sessions/ranking.rs`의 `order_after_router_orders_by_probability_and_keeps_low`, `order_after_router_same_probability_follows_rrf_order` |
| 항목의 남김 확률은 호출과 결과 중 큰 값이다. | `saturn-terminal/core/src/routers/tests.rs`의 `compact_verdicts_takes_larger_of_call_and_result` |
| 답이 없는 항목(실패한 조각 포함)은 RRF 순으로 뒤에 둔다. | `saturn-terminal/core/src/sessions/ranking.rs`의 `order_after_router_unanswered_follow_answered_in_rrf_order`, `saturn-terminal/core/src/routers/tests.rs`의 `compact_verdicts_merges_pieces_and_skips_failed_piece` |
| router가 전부 답하지 못하면 RRF 순서로 예산까지 채운다. | `saturn-terminal/core/src/sessions/ranking.rs`의 `order_after_router_no_verdicts_keeps_rrf_order`, `saturn-terminal/core/src/sessions/packet/tests.rs`의 `build_packet_router_no_response_fills_in_rrf_order` |
| 띄어쓰기와 조사가 달라도 같은 한글 조각을 만든다. | `saturn-terminal/core/src/sessions/fragments.rs`의 `fragments_spacing_and_particle_share_hangul_bigrams` |
| 영문 식별자는 식별자 경계에서 나눈다. | `saturn-terminal/core/src/sessions/fragments.rs`의 `fragments_split_text_into_search_fragments` |
| 자모로 풀린 한글도 음절 한글과 같은 조각을 만든다. | `saturn-terminal/core/src/sessions/fragments.rs`의 `fragments_nfd_hangul_matches_nfc` |
| 같은 결과에서 늘 같은 메모를 만든다. | `saturn-terminal/core/src/sessions/memo.rs`의 `tool_memo_same_result_gives_same_memo` |
| 순위, Jev, 작업 LLM이 같은 후보 집합과 같은 예산을 받고 선택 번호만 다르다. | `saturn-terminal/core/src/sessions/evidence/tests.rs`의 `the_three_methods_get_the_same_set_and_budget_and_differ_only_in_ids` |
| 후보 집합은 입력 순서에 따르지 않고, 없는 번호, 다른 폴더, 바뀐 해시를 가른다. 같은 번호는 만들지 않는다. | 같은 파일의 `set_hash_ignores_input_order_and_follows_any_candidate_change`, `set_rejects_a_duplicate_id`, `resolve_checks_the_id_the_folder_and_the_hash` |
| Jev와 작업 LLM이 판정하지 못하면 순위 선택으로 돌아가고 같은 확률은 순위 순이다. | 같은 파일의 `a_selection_that_cannot_be_decided_falls_back_to_the_rank_selection`, `jev_ties_follow_rank_order_and_candidate_input_order_changes_nothing` |
| 예산에 안 드는 후보는 건너뛰고, 순위에 없는 후보는 번호가 큰 순으로 뒤에 붙는다. | 같은 파일의 `a_candidate_over_the_remaining_budget_is_skipped_and_smaller_ones_still_fit`, `rank_order_drops_unknown_ids_and_appends_unranked_candidates_newest_first` |
| 검색은 검색어 순위로 돌려주고 읽기는 기록 원문을 쪽 단위로 돌려준다. | `saturn-terminal/engine/src/lifecycle/evidence.rs`의 `search_ranks_by_the_query_and_read_returns_the_recorded_text_in_pages` |
| 같은 번호의 입력, 글, 도구 호출은 종류별 참조로 각자의 원문을 읽고, 옛 번호는 도구 호출만 가리킨다. 하위 에이전트 글은 후보가 아니다. | `saturn-terminal/engine/src/lifecycle/evidence.rs`의 `an_input_and_an_answer_with_the_same_number_are_read_by_their_own_kind`, `an_input_and_a_tool_record_with_the_same_number_stay_apart_and_legacy_reads_the_tool` |
| 바뀐 원문이나 없는 해시는 `Stale`, 다른 채팅의 참조는 `NotFound`다. | `saturn-terminal/engine/src/lifecycle/evidence.rs`의 `a_changed_text_or_a_missing_hash_is_refused_as_stale`, `a_reference_to_another_chat_is_refused_and_other_chat_dialogue_is_not_listed` |
| 읽기 권한이 좁아지면 전에 나온 도구 호출 참조는 `Scope`이고, 회수된 출입증은 검색도 읽기도 거절하며, 비밀이 든 글은 후보도 읽기도 아니다. | `saturn-terminal/engine/src/lifecycle/evidence.rs`의 `a_narrowed_read_permission_refuses_a_reference_it_listed_before`, `a_revoked_pass_refuses_search_and_typed_reads_alike`, `dialogue_with_a_router_key_is_neither_listed_nor_readable` |
| 명령줄은 옛 번호와 종류별 참조를 둘 다 읽고, 입력과 글 후보는 읽을 수 있는 참조로 출력한다. | `saturn-terminal/cli/src/args.rs`의 `evidence_read_takes_a_legacy_number_or_a_typed_reference`, `saturn-terminal/cli/src/commands/evidence.rs`의 `dialogue_candidates_and_typed_reads_carry_the_full_reference`, `a_legacy_number_sends_no_kind_and_a_reference_sends_kind_chat_and_hash` |
| 없는 번호, 바뀐 해시, 읽기 범위 밖과 읽기 규칙이 거부하는 경로의 기록은 거절하고 후보에서 빼며, 다른 채팅의 기록은 나오지 않는다. | 같은 파일의 `read_refuses_unknown_other_chat_stale_and_out_of_scope_records` |
| 출입증이 없으면 거절하고 조회마다 기록한다. 닿지 못한 시도는 도구 결과의 오류 첫머리 표지로 `Unreachable`이 되고, 첫머리가 아닌 표지는 세지 않는다. | 같은 파일의 `an_unknown_pass_is_refused_and_every_lookup_is_counted` |
| 안내 한 줄은 옵션을 켜고 원문이 잘렸을 때만 붙는다. | `saturn-terminal/core/src/sessions/packet/tests.rs`의 `build_packet_lookup_hint_only_when_the_option_is_on_and_an_original_was_cut` |
| 순위가 router 전체 판단과 얼마나 겹치는지 잰다. | [RRF k와 router 상위 N 실험 결과](../experiments/rrf-k-top-n/report.md): 상위 10개 12.0%, 상위 40개 44.5% |
| 단어 조각 단위가 오타 입력에서 관련 후보를 놓치지 않는다. | [단어 조각 단위별 오타 재현율 실험 결과](../experiments/wordpiece-typo-recall/report.md) |
| router 없이 순위로 채운 패킷은 router 전체 판단 패킷보다 정답률이 10%p를 넘게 낮지 않다. | [새 패킷 규칙의 전환 품질 재측정](../experiments/handoff-packet-quality-v2/report.md): 정답률 차이 +50.7%p [44.7, 56.7]로 기각. 판단 없는 패킷의 규칙은 [router 실패](router.md#router-실패)에 있다. |
| router가 시작한 전환은 `compact` 판단이 실패하면 건너뛰고, 강제한 전환은 순위 순서로 채운다. | `saturn-terminal/core/src/routers/failure.rs`의 `compact_failure_skips_router_transition_and_fills_forced_one`, `saturn-terminal/core/src/sessions/ranking.rs`의 `order_after_router_no_verdicts_keeps_rrf_order` |

## 단점

- router가 실패해 순위 순서로 채운 패킷은 정답률이 20.6%로 패킷 없음(20.0%)과 같은 수준이다([재측정 결과](../experiments/handoff-packet-quality-v2/report.md)). 그래서 router가 시작한 전환은 건너뛰고, 강제한 전환만 이 패킷으로 채운다.
- 순위 채널은 같은 뜻의 다른 말을 모르므로 router가 모두 실패하면 대체 순서에서 같은 뜻의 후보를 놓칠 수 있다.
- RRF 상위 40개는 router 전체 판단이 남긴 항목의 절반 이상을 놓친다([실험 결과](../experiments/rrf-k-top-n/report.md)).
- 영문 단어 사이 공백이 빠지면 소문자 단어가 하나로 붙어 단어 겹침을 놓친다. 이 오타의 상위 10개 재현율은 56.4%였다([실험 결과](../experiments/wordpiece-typo-recall/report.md)).
- 후보가 150개면 질문이 300개라 router 입력 토큰이 후보 수에 비례한다. 큰 요청은 여러 건으로 나뉘고, 병렬로 보내면 지연은 조각 수와 거의 무관하게 약 0.3초다([#179](https://github.com/woonyong-choi/saturn/issues/179) 실측).
- 나뉜 요청은 조각마다 같은 state를 보내 입력 토큰이 조각 수만큼 늘고, 조각이 실패하면 그 항목은 순위로만 정해진다.
- 동시 요청이 8개를 넘을 때의 속도 제한은 재지 않았다.
- 기준 파일 범위를 실측으로 맞춰야 한다.

## 대안

- RRF 상위 N개만 router에 묻는 방식은 router가 남길 항목을 상위 10개에서 12.0%, 상위 40개에서도 44.5%만 담아 버렸다([결정 기록](../decisions/2026-10-01-router-all-candidates.md)).
- 채널 점수의 가중합은 단위가 다른 점수의 가중치를 따로 학습해야 해 버렸다([결정 기록](../decisions/2026-10-01-ranked-candidates-before-router.md)).
- 남김 확률 0.5 이상만 경쟁 구역에 넣는 방식은 근거 항목의 88.0%를 버려 버렸다([결정 기록](../decisions/2026-10-02-fill-packet-by-probability.md)).
- 임베딩 채널을 기본으로 넣는 방식은 같은 뜻 질의에서 이득이 없고 단어가 겹치는 질의의 재현율과 설치 크기, 상주 메모리를 잃어 버렸다([결정 기록](../decisions/2026-10-01-router-decides-synonyms.md), [실험 보고서](../experiments/embedding-synonym/report.md)).
- 입력마다 LLM으로 사실 문장을 뽑는 방식은 호출과 출력 비용이 들고 원문 대신 생성문을 저장해 버렸다.
