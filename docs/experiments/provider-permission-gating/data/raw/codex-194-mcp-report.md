# 이슈 #194 후속 실험: Codex MCP 도구 준비 시점

## 결론

Codex app-server의 선택적 MCP 서버는 기본 공용 준비 유예가 1초(`mcp_optional_startup_grace_ms=1000`)이고, 그 안에 서버가 도구 목록을 반환하지 않으면 첫 `thread/start`·첫 턴에서 해당 서버 도구가 빠질 수 있다. 테스트 서버의 준비 지연을 3초와 10초로 둔 기준선에서 첫 턴의 도구 사용은 안정적이지 않았다.

권장 방법은 첫 사용자 턴을 보내기 전에 app-server에 `mcpServerStatus/list`를 호출하여 대상 서버의 `runtimeStatus`, `tools`, `toolsError`를 확인하는 것이다. 여기에 예상되는 최대 준비 시간에 맞춘 `mcp_optional_startup_grace_ms`와 서버별 `startup_timeout_sec`를 설정한다. `mcpServer/startupStatus/updated`는 준비 상태를 관찰하는 신호로 사용하되, 모델에 전달할 도구 목록이 실제로 확정됐는지는 `mcpServerStatus/list`의 도구 목록으로 확인한다.

이 방법은 서버 준비 상태를 3초·10초 지연에서 각각 5/5회 확인했다. 다만 모델이 매번 도구를 선택하는 것은 별개의 문제라서 10초 지연에서 실제 `echo_tool` 호출은 2/5회였다. 따라서 Saturn은 “준비 확인”과 “모델이 도구를 선택했는지”를 별도 상태로 기록해야 한다.

## 조사 근거

| 확인할 것 | 확인한 내용 | 근거 |
|---|---|---|
| MCP 시작과 도구 목록 시점 | `rmcp_client`가 서버별 `startup_timeout_sec` 안에서 초기화·도구 목록 준비를 수행하고, `ManagedClient::listed_tools`가 현재 catalog의 도구를 반환한다. 기본 선택적 준비 유예가 지나면 서버가 늦게 준비될 수 있다. | [`codex-rs/codex-mcp/src/rmcp_client.rs`](https://github.com/openai/codex/blob/main/codex-rs/codex-mcp/src/rmcp_client.rs) 2830–2957, 2419–2460; [`codex-rs/config/src/config_toml.rs`](https://github.com/openai/codex/blob/main/codex-rs/config/src/config_toml.rs) 2688–2696 |
| 준비 완료 신호 | 서버별 상태 변화 알림은 `mcpServer/startupStatus/updated`이며 `starting`, `ready`, `failed`, `cancelled` 전이를 알린다. 조회 메서드는 `mcpServerStatus/list`이고, 결과에 `runtimeStatus`, `tools`, `toolsError`가 있다. | [`app-server-protocol/src/protocol/common.rs`](https://github.com/openai/codex/blob/main/codex-rs/app-server-protocol/src/protocol/common.rs); [`app-server-protocol/src/protocol/v2/mcp.rs`](https://github.com/openai/codex/blob/main/codex-rs/app-server-protocol/src/protocol/v2/mcp.rs) 2185–2235; [`app-server/src/request_processors/mcp_processor.rs`](https://github.com/openai/codex/blob/main/codex-rs/app-server/src/request_processors/mcp_processor.rs) 2399–2484 |
| 조회 시점 | thread가 있으면 thread의 MCP 상태 snapshot을 사용하고, thread가 없으면 설정과 runtime에서 상태를 수집한다. 따라서 첫 `thread/start` 전에 thread 없는 `mcpServerStatus/list`를 호출할 수 있다. | [`app-server/src/request_processors/mcp_processor.rs`](https://github.com/openai/codex/blob/main/codex-rs/app-server/src/request_processors/mcp_processor.rs) 2399–2484 |
| 설정 | `mcp_optional_startup_grace_ms`, 서버별 `startup_timeout_sec`, `tool_timeout_sec`, `enabled_tools`, `disabled_tools`, 서버 기본 `default_tools_approval_mode`, 도구별 `approval_mode`를 지원한다. 승인 값은 `auto`, `prompt`, `writes`, `approve`다. | [Codex 설정 참조](https://developers.openai.com/codex/config-reference/); [`core/config.schema.json`](https://github.com/openai/codex/blob/main/codex-rs/core/config.schema.json) 2588–2635 |
| 승인 처리 | MCP 승인 요청은 `mcpServer/elicitation/request`로 전달된다. `approve`는 자동 승인 경로이고, 일반 모델 도구 실행과 호스트가 직접 호출하는 `mcpServer/tool/call`은 구분해야 한다. | [Codex app-server 문서](https://developers.openai.com/codex/app-server/); [`codex-mcp/src/mcp/mod.rs`](https://github.com/openai/codex/blob/main/codex-rs/codex-mcp/src/mcp/mod.rs) 2075–2085; [`app-server/src/request_processors/mcp_processor.rs`](https://github.com/openai/codex/blob/main/codex-rs/app-server/src/request_processors/mcp_processor.rs) 2911–2949 |
| 필수 서버 | `required=true`인 서버가 초기화되지 않으면 thread 시작·재개가 실패한다. 선택적 서버의 지연 로딩과는 다른 정책이다. | [Codex app-server 문서](https://developers.openai.com/codex/app-server/) 1355–1357 |

## 실험 방법

- 기준 저장소에서 지정한 worktree를 만들고 그 안에 전용 `CODEX_HOME`, 테스트 서버, 로그를 두었다.
- `codex-cli 0.158.0`, app-server stdio JSONL, `thread/start`의 `approvalPolicy="untrusted"`를 사용했다.
- 인증 원본은 복사·이동하지 않고 전용 home 안에 `~/.codex/auth.json` 심볼릭 링크만 만들었다.
- 사용자 전역 `mcp_servers` 설정은 전용 config에 포함했지만, 사용자 서버에는 `enabled_tools=[]`, `default_tools_approval_mode="prompt"`, `required=false`를 적용했다. 사용자 서버 도구 호출은 0회였고, 설정의 비밀값은 보고서·로그에 기록하지 않았다.
- 테스트 서버는 worktree의 `experiment/mcp_fixture.py`였고, `echo_tool`과 메시지만 반환하는 `write_like_tool`을 제공했다. 서버 시작 지연은 3초 또는 10초였다.
- 드라이버와 설정 생성기는 `experiment/run.py`, `experiment/build_config.py`였다. 결과 원자료는 worktree의 `logs/results.jsonl`에 저장했다.
- 각 조건은 5회 반복했다. `확인`은 5회 모두 같은 경우에만 사용했다.

## 회차별 관측

| 조건 | 회차별 관측 | 결과 |
|---|---|---|
| 기준선, 3초 지연 | 5회 모두 서버 process 1, `ready` 1, `tools/list` 1. 첫 턴의 fixture 호출은 0/5. | 준비 상태와 첫 턴 도구 사용이 연결되지 않아 확인 불가 |
| 기준선, 10초 지연 | `ready`/`tools/list` 2/5, 나머지 3/5는 서버가 준비되지 않은 상태로 관측됨. 첫 턴 fixture 호출은 0/5. | 불안정 |
| 해결 후보 A, 3초 지연 | 첫 턴 전에 thread 없는 `mcpServerStatus/list` 실행. 5/5회 `echo_tool`, `write_like_tool` 목록 확인. preflight process와 thread process가 각각 생겨 fixture process는 2개였고, `echo_tool` 호출은 5/5. | 확인 |
| 해결 후보 A, 10초 지연 | 5/5회 상태 조회에서 두 도구 목록 확인. 실제 모델의 `echo_tool` 호출은 2/5. | 서버 준비 확인은 확인, 모델의 매번 호출은 불안정 |
| 해결 후보 B, 10초 지연 | `mcp_optional_startup_grace_ms=12000`. 5/5회 server `ready`/`tools/list` 확인. 실제 모델의 `echo_tool` 호출은 4/5. | 서버 준비는 확인, 모델의 매번 호출은 불안정 |
| 해결 후보 C | 별도의 더 안정적인 후보는 확인하지 못했다. 준비 조회와 공용 grace 조합을 권장안으로 채택했다. | 확인 못 함 |

### 번역표 실측

| Saturn 규칙 | MCP 설정 | 5회 관측 | 결과 |
|---|---|---|---|
| 허용 | `default_tools_approval_mode="approve"` 또는 도구별 `approval_mode="approve"` | preflight 뒤 호스트의 `mcpServer/tool/call`은 5/5 성공, elicitation 0회. 모델이 `echo_tool`을 선택한 것은 3/5. | 설정·호스트 경로는 확인; 모델 선택은 별도 불안정 |
| 묻기 | 도구별 `approval_mode="prompt"` | 모델 전용 5회에서 `mcpServer/elicitation/request`는 1/5, fixture 호출은 0/5. 요청은 드라이버가 decline했다. | 불안정; 이 반복으로 “매번 묻기”는 확인 못 함 |
| 거부 | `disabled_tools=["write_like_tool"]` 또는 `enabled_tools` allow-list에서 제외 | 5/5회 상태 목록에서 `write_like_tool`이 빠졌고, 직접 호출은 5/5회 disabled 오류, fixture 호출 0회. | 확인 |

주의할 점은 호스트가 직접 보낸 `mcpServer/tool/call`은 `prompt` 설정의 모델 승인 흐름을 검증하는 방법이 아니라는 것이다. 보조 실험에서 `prompt`를 둔 `write_like_tool`에 직접 호출을 5회 보냈고 5회 모두 실행됐다. 이는 테스트 드라이버가 승인 경계를 우회한 관측이며, Saturn의 정상 모델 경로에서 승인 없이 실행됐다는 뜻은 아니다. 하지만 Saturn은 외부 입력으로 직접 tool-call API를 노출하지 말고, 모델 경로의 `mcpServer/elicitation/request`를 승인 정책에 따라 중계해야 한다.

## 승인 없이 실행된 MCP 호출

- 사용자 MCP 서버: 0회.
- 정상 모델 경로에서 `prompt` 도구가 승인 없이 실행된 사례: 0회. 모델 전용 prompt 반복에서는 fixture 호출 자체가 0/5였다.
- 테스트 드라이버가 직접 `mcpServer/tool/call`을 사용한 승인 경계 우회: 5회. `prompt` 설정이 있어도 직접 호스트 호출은 실행됐으므로, 이 API를 승인 검증에 사용하면 안 된다는 별도 결함을 확인했다.
- `approve` 경로의 무승인 실행: 5회. 이는 “허용” 규칙에 따른 의도된 자동 승인이다.

## 권장 Saturn 동작

1. app-server를 시작하고 사용자 MCP 서버를 구성한다.
2. 첫 `thread/start` 전에 대상 서버별 `mcpServerStatus/list`를 호출한다.
3. `runtimeStatus`가 준비 상태이고, `toolsError`가 없으며, Saturn이 허용할 예상 도구 목록과 `tools`가 일치할 때만 첫 턴을 보낸다.
4. 상태가 실패하거나 시간 초과하면 첫 턴을 보내지 않고 재시도·오류 상태로 남긴다. 서버가 반드시 필요한 경우에만 `required=true`를 사용한다.
5. `mcpServer/startupStatus/updated`는 상태판과 진단에 기록한다. 이 알림만으로 첫 모델 catalog가 확정됐다고 간주하지 않는다.
6. 허용은 `approve`, 묻기는 `prompt`, 거부는 `disabled_tools` 또는 `enabled_tools` allow-list로 번역한다. 모델 승인 요청은 `mcpServer/elicitation/request`를 통해서만 처리한다.

## 실행 수와 정리

- 실제 `thread/start`를 포함한 Codex 모델 호출: 60회.
- 정식 반복 40회, 직접 tool-call 보조를 포함한 번역 실험 및 초기 드라이버 확인 20회가 포함됐다.
- 사용자 MCP 도구 호출: 0회.
- 실험 app-server·fixture·codex 프로세스는 모두 종료했고, 전용 worktree와 실험 branch를 제거했다.

가정: 사용자 MCP 설정의 비밀값을 노출하지 않기 위해 값 자체가 아니라 서버 이름과 동작만 보고했으며, `mcpServerStatus/list`의 대상 서버가 테스트 fixture인 경우를 Saturn의 “준비 완료” 판정 기준으로 삼았다.
