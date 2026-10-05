# Claude 확장 도구와 Jev의 적용 경계

## 질문

[영상](https://www.youtube.com/watch?v=oDW7XUrzork)의 다섯 도구를 Saturn에 적용하면 Jev가 필요 없어지는지 판단한다. 영상의 자동 자막과 고정 댓글의 [작성자 가이드](https://lazyowen.com/en/guides/claude-skills-top5-0815)에서 이름을 확인하고 각 공식 소스의 역할을 대조했다. 확인일은 2026-10-06이다. 설치나 실행을 안내하는 외부 문장은 연구 자료로만 읽었고 도구를 설치하거나 기존 설정을 변경하지 않았다.

## 출처

| 도구 | 고정한 소스 | 핵심 근거 |
|---|---|---|
| OmniRoute | [23a1148](https://github.com/diegosouzapw/OmniRoute/tree/23a11484862b3bb589a55e85b00e4ac53ffeb234) | [후보 점수](https://github.com/diegosouzapw/OmniRoute/blob/23a11484862b3bb589a55e85b00e4ac53ffeb234/open-sse/services/autoCombo/scoring.ts), [자동 조합](https://github.com/diegosouzapw/OmniRoute/blob/23a11484862b3bb589a55e85b00e4ac53ffeb234/docs/routing/AUTO-COMBO.md) |
| Headroom | [2651489](https://github.com/headroomlabs-ai/headroom/tree/2651489745937e1479af3e92151f0f811ccfafda) | [내용별 압축](https://github.com/headroomlabs-ai/headroom/blob/2651489745937e1479af3e92151f0f811ccfafda/docs/content/docs/how-compression-works.mdx), [원문 복구](https://github.com/headroomlabs-ai/headroom/blob/2651489745937e1479af3e92151f0f811ccfafda/docs/content/docs/ccr.mdx) |
| Claude-mem | [1bb6439](https://github.com/thedotmack/claude-mem/tree/1bb64393a6122c2cdf45cf219cd9b4d1cf78645b) | [단계별 검색](https://github.com/thedotmack/claude-mem/blob/1bb64393a6122c2cdf45cf219cd9b4d1cf78645b/plugin/skills/mem-search/SKILL.md), [관찰 생성 provider](https://github.com/thedotmack/claude-mem/blob/1bb64393a6122c2cdf45cf219cd9b4d1cf78645b/src/server/generation/providers/ClaudeObservationProvider.ts) |
| Claude Code Setup | [d182ca4](https://github.com/anthropics/claude-plugins-official/tree/d182ca456ca09d31d139f7d3818d1d333b103cce/plugins/claude-code-setup) | [추천 스킬](https://github.com/anthropics/claude-plugins-official/blob/d182ca456ca09d31d139f7d3818d1d333b103cce/plugins/claude-code-setup/skills/claude-automation-recommender/SKILL.md) |
| Task Observer | [da86026](https://github.com/rebelytics/one-skill-to-rule-them-all/tree/da860265ae968bdc6bb13b96c2c75fcb61176b72) | [관찰 스킬](https://github.com/rebelytics/one-skill-to-rule-them-all/blob/da860265ae968bdc6bb13b96c2c75fcb61176b72/SKILL.md), [검토와 적용](https://github.com/rebelytics/one-skill-to-rule-them-all/blob/da860265ae968bdc6bb13b96c2c75fcb61176b72/references/weekly-review.md) |

## 비교

### OmniRoute

API gateway이며 스킬이 아니다. 요청 종류 적합성, 잔여 한도, 장애 상태, 가격, 지연, session 가용성 등을 코드의 가중 점수로 합쳐 후보를 고른다. 단순한 quota fallback과 task-aware 선택은 구분한다. task fit 점수는 실제 특정 작업의 정답을 증명하지 않는다. 문서에는 Codex와 Claude Code 연결이 모두 있다.

Saturn에 가져올 것은 capability·한도·관측 지연·실패 이력의 후보 필터와 점수다. 이런 값은 Jev에 다시 물을 필요가 없다. 작업 의미가 불명확할 때의 분류만 선택적으로 Jev와 비교한다. 무료 모델 수와 광고상 무료 토큰 합계는 계정별 사용 가능량이나 품질 보장이 아니다. auto category 필터의 fail-open과 [strict zero cost](https://github.com/diegosouzapw/OmniRoute/blob/23a11484862b3bb589a55e85b00e4ac53ffeb234/docs/routing/STRICT_ZERO_COST.md)를 구분해야 한다. Saturn의 명시적 무료 제한을 후보 없음으로 완화하면 안 된다.

외부 gateway의 자동 선택과 Saturn의 자동 선택을 동시에 소유하지 않는다. gateway에 고정 모델을 넘기거나, gateway가 선택했다는 사실과 실제 모델을 기록한다. unknown delivery의 재전송이나 도구 실행 권한은 gateway 점수로 결정하지 않는다.

### Headroom

형식에 따른 구조적 압축과 원문 재조회가 핵심이다. JSON, 로그, 검색 결과, diff, 코드에 각각 다른 압축기를 사용한다. 새 요청의 델타를 압축하고 과거 prefix를 유지하는 cache mode는 매번 전체 이력을 다시 선별하는 방식과 다르다.

Jev를 쓰지 않는다고 모델 추론이 전혀 없는 것은 아니다. [Kompress](https://github.com/headroomlabs-ai/headroom/blob/2651489745937e1479af3e92151f0f811ccfafda/headroom/transforms/kompress_compressor.py)는 ModernBERT 기반 token compressor이며 ONNX나 PyTorch 실행 경로를 가진다. 내용 감지는 Magika와 패턴을 함께 사용할 수 있다. 구조적 경로와 학습 모델 경로의 CPU·메모리·지연을 따로 측정해야 한다.

Saturn에는 원문 ID와 생략 표시를 남기고 필요한 원문을 다시 가져오는 구조, 형식별 무손실 축약, cache prefix 보존을 우선 검토한다. 원문이 저장돼 있다는 사실과 모델이 실제로 필요한 원문을 다시 찾는다는 사실은 다르다. CCR 복구 도구는 proxy와 SDK 경로의 지원 범위도 다르다. 이전 도구 사용 금지 기억 실험은 재조회가 있는 Headroom 평가를 대신할 수 없다. 압축된 입력·복구 요청·복구 응답·추가 모델 턴·캐시 손실의 합계를 비교해야 한다.

### Claude-mem

세션의 관찰을 저장하고 검색 가능한 요약과 다음 세션 맥락을 만든다. 현재 공식 소스는 Claude Code 외 Codex 등 다른 환경도 안내하므로 Claude 전용으로 단정하지 않는다. 검색은 작은 색인, 주변 시점, 선택한 관찰, 필요한 경우 원시 도구 출력 순으로 확장한다.

관찰 생성 provider에는 LLM 호출과 usage 기록이 있다. 추가 Jev가 없다는 사실이 추가 모델 비용 0을 뜻하지 않는다. 검색을 선택하는 주체도 작업 LLM일 수 있다. 저장 시 일부 원시 도구 본문은 길이 한도로 잘리므로 모든 원문이 영구적으로 완전 보존된다고 간주하지 않는다.

Saturn에 가져올 것은 작은 색인부터 읽고 부족한 부분만 확장하는 방법이다. 원본 기록 저장소를 정본으로 유지하고 파생 요약·색인은 버전과 출처를 가진 재생성 가능한 자료로 둔다. 두 기억 저장소를 무조건 함께 붙이지 않는다. LLM이 이미 만든 요약·후보의 범위 선별에 Jev를 추가하는 이득은 별도로 측정한다.

### Claude Code Setup

프로젝트 파일을 읽고 hooks, skills, MCP, subagent 등 자동화를 추천하는 공식 플러그인이다. 소스 스킬은 읽기 전용 추천이며 스스로 설치·수정하지 않는다고 명시한다. 매 요청의 모델·맥락 router가 아니다.

Saturn에는 프로젝트 최초 접수나 의존성 변경 때만 capability 목록을 만드는 방식이 맞다. Cargo.toml, 테스트 명령, 설정 유무처럼 정적인 값은 코드로 찾는다. 의미를 해석해야 할 추천은 이미 쓰는 설계 LLM의 작업으로 다룬다. 매 입력마다 Jev나 추천 LLM을 추가 호출할 이유가 없다.

### Task Observer

작업 중 반복되는 패턴, 수정, 실패 원인, 방법을 관찰 파일에 남기고 스킬 변경으로 연결한다. 가중치 학습 모델이 아니라 작업 LLM에 주는 관찰·검토 절차다. 대화형 검토와 사용자가 부재한 예약 실행의 적용 정책이 다르며, 변경 종류에 따라 별도 검토·시험 경로가 있다. 사용자의 교정 외 자율 실행에서 발견한 문제도 관찰하므로 교정만 쓰는 도구라고 단정하지 않는다.

Saturn에는 실행 사실·실패·재작업을 decision_id에 연결하는 관찰 구조를 가져온다. 사용자 교정을 정상 운영의 필수 라벨로 요구하지 않는다. 관찰 하나로 권한·제약·라우팅 규칙을 자동 승격하지 않고, 후보 변경과 검증된 적용을 분리한다. 원 스킬 전체와 참조 파일을 매 입력에 주입하면 절약을 보장할 수 없다. 트리거·작은 색인·필요한 참조만 제공하고 호출 및 읽기 토큰을 계산한다.

## 적용 판단

| 항목 | 판단 | 이유 |
|---|---|---|
| capability·quota·실패·지연 후보 필터 | 코드로 처리 | 관측값에 적용할 결정적 규칙 |
| 프로젝트별 스킬·도구 목록 | 최초 접수와 변경 시 구성 | 모든 입력에서 재탐색할 필요 없음 |
| 원문 보존·단계별 검색·재조회 | Saturn 구조에 반영할 후보 | 전체 이력 재주입을 줄이면서 근거 복구 가능 |
| 형식별 무손실 축약·cache prefix 유지 | 먼저 비교할 기준선 | 의미 판단 호출 없이 줄일 수 있는 부분 |
| 기존 LLM의 제약·작업 해석 | 유지 | 기존 작업 모델의 의미 해석 활용 |
| Jev 후보 선별 | 효과가 확인된 불확실한 선택에만 검토 | [예비 결과](report.md)의 역할별 추가 이득 불확실 |
| 다섯 도구 일괄 설치 | 채택 근거 없음 | gateway·기억 저장·압축 책임과 비용의 중복 가능성 |
| Jev 전면 제거 | 채택 근거 없음 | 단순 기준선과 의미 선별의 역할 구분 필요 |

## 후속 비교 조건

스킬은 작업 절차이고 Jev는 선택기이므로 둘을 하나의 대체재로 비교하지 않는다. 동일한 작업 LLM과 기록·도구 예산에서 아래 조건을 비교한다. 이번 예비 실험 수집 뒤 추가된 설계이며 기존 H1~H3의 확인 분석에 합치지 않는다.

| 조건 | 작업 절차 | 선택 판단 |
|---|---|---|
| A | 현재 Saturn 실험 기준선 | 단계 고정·코드 필터·검색 |
| B | 필요한 스킬 절차와 단계별 원문 조회 | 코드 필터·검색, 추가 Jev 없음 |
| C | B와 동일 | 불확실한 후보에만 Jev |
| D | B와 동일 | 모든 후보에 Jev |

설계 Opus, 검증 Sol 또는 Astra, 구현 Sonnet을 공통 기준으로 둔다. 낮은 모델 비교는 같은 과제의 실제 결과를 대조한다. 스킬 로딩 방식이 달라졌다고 실행 모델까지 함께 바꾸면 원인을 구분할 수 없다.

먼저 역할별로 분리해 측정하고 마지막에 조합한다. 라우팅은 가용성 장애 fixture와 실제 정답 과제를 나눈다. 압축은 원문 그대로, 형식별 무손실 압축, Headroom 기본값, Jev 선별, 형식별 압축과 선택적 Jev를 대조한다. 원문 재조회는 모든 조건에 같은 도구와 권한으로 허용하고, 재조회 금지 기억력 시험은 별도 층으로 둔다. 기억은 정본 색인과 요약 생성·주입 비용을 함께 계산한다. 스킬 개선은 같은 미래 과제 묶음에서 변경 전후를 대조하고 오염되지 않은 평가셋을 유지한다.

성공당 총토큰은 작업 입력·출력, 스킬 본문과 참조, 관찰·요약 생성, Jev state, 원문 복구, 재시도·재작업의 합계다. provider마다 토큰 단위가 다르므로 합계와 provider별 값을 함께 보고한다. cache read·write와 uncached 입력도 따로 기록한다. 무료 한도와 USD 비용은 합치지 않는다. 도구 오류를 모델의 의미 오류로 채점하지 않는다.

B가 A보다 품질을 유지하며 적게 사용하면 그 구조를 채택한다. C가 B보다 추가 이득이 없으면 그 역할에서는 Jev를 호출하지 않는다. C가 D와 동등한 품질로 더 적게 쓰면 선택적으로 호출한다. 이 판정은 원 구현을 실제 연결한 동일 과제 실험으로 확인해야 하며 소스 분석이나 기존 예비 결과로 대신하지 않는다.
