"""데모 2: 장애 지휘. 알림 폭주에서 근본 원인 서비스를 찾고 공지 초안을 검증한다.

step 1  묶기와 분류: 알림마다 "같은 장애인가" noul, 장애 종류 choice, 고객 영향 score.
        비교용으로 전체 알림을 보고 원인 서비스를 한 번에 고르는 추측 질문도 함께 묻는다
step 2  계층 내려가기 1층: 같은 장애로 묶인 알림만 state에 넣고 도메인을 고른다
step 3  계층 내려가기 2층: 상위 도메인 3개(경로 3개)의 서비스를 한 호출에 병렬로 묻는다.
        경로 점수 = 간선 확률의 기하평균
step 4  검증 뒤 올리기: LLM이 쓴 상태 페이지 초안을 앞 판단과 대조하고 최댓값으로 보류를 정한다
"""

from chain import Chain, choice, max_gate, noul, path_score, run_demo, score

BEAM = 3
SAME_INCIDENT = 0.5
HOLD_AT = 0.7

SERVICES = {
    "edge": {
        "cdn": "Content delivery and TLS termination for static assets.",
        "api-gateway": "Public API entry point; routes /v1/* to backend services.",
        "waf": "Web application firewall and bot rules.",
    },
    "identity": {
        "auth-service": "Login, password checks, token issuing.",
        "session-store": "Session tokens storage.",
        "sso-bridge": "SAML and OIDC federation for enterprise customers.",
    },
    "commerce": {
        "checkout": "Cart to order flow.",
        "payments": "Card tokenization and charge capture.",
        "inventory": "Stock levels and reservations.",
        "pricing": "Prices, discounts, tax.",
    },
    "data": {
        "orders-db": "Primary Postgres cluster for orders and inventory.",
        "kafka": "Event bus.",
        "redis-cache": "Shared cache.",
        "search-index": "Product search cluster.",
    },
    "platform": {
        "dns": "Internal and external DNS.",
        "k8s-control-plane": "Kubernetes API and scheduling.",
        "config-service": "Feature flags and runtime configuration pushes.",
    },
}
DOMAIN_OF = {svc: domain for domain, svcs in SERVICES.items() for svc in svcs}
DOMAINS = {
    "edge": "Traffic entry: CDN, gateway, firewall.",
    "identity": "Authentication and sessions.",
    "commerce": "Checkout, payments, inventory, pricing business services.",
    "data": "Databases, cache, event bus, search.",
    "platform": "DNS, Kubernetes, configuration and feature flags.",
}
KINDS = {
    "bad_deploy": "A code deploy introduced the failure.",
    "config_change": "A configuration or feature flag change introduced the failure.",
    "infrastructure_failure": "A database, network, or node failure.",
    "capacity": "Load exceeded capacity without a change or failure.",
    "security": "An attack or abuse pattern.",
    "other": "None of the above.",
}


def alert(t: str, service: str, severity: str, summary: str) -> dict:
    return {"time": t, "service": service, "severity": severity, "summary": summary}


CASES = [
    {
        "id": "flag-push",
        "title": "기능 플래그 배포가 결제를 깨고 증상은 게이트웨이에서 먼저 보임",
        "alerts": [
            alert("14:03:10", "api-gateway", "critical", "5xx rate 12% on POST /v1/checkout"),
            alert("14:02:55", "payments", "major", "tokenizer error rate 38%: InvalidKeyFormat"),
            alert("14:03:40", "checkout", "major", "order completion p99 9.1 s, timeouts calling payments"),
            alert("14:04:02", "kafka", "minor", "topic payments.events producer retries rising"),
            alert("14:01:30", "search-index", "minor", "reindex job lagging 25 min behind"),
            alert("14:00:00", "cdn", "warning", "TLS certificate for static.example.com expires in 20 days"),
            alert("14:03:58", "redis-cache", "minor", "hit rate dropped from 94% to 88% on checkout keys"),
            alert("14:02:10", "k8s-control-plane", "warning", "node ip-10-3-7-21 disk pressure (batch pool)"),
        ],
        "changes": [
            {"time": "14:02:00", "service": "config-service", "change": "push cfg-8812: payments.new_tokenizer=true for 100%"},
            {"time": "11:40:00", "service": "search-index", "change": "deploy search-indexer v3.2.1"},
        ],
        "draft": "Some customers may see errors when completing checkout. We have identified the cause and are rolling back a recent change. Next update in 30 minutes.",
        "expected": {
            "cluster": [0, 1, 2, 3, 6],
            "kind": "config_change",
            "root": "config-service",
            "publish": True,
        },
    },
    {
        "id": "db-failover",
        "title": "주 DB 장애 전환 뒤 복제 지연, 겉으로는 재고와 결제 문제처럼 보임",
        "alerts": [
            alert("09:12:20", "checkout", "critical", "order creation failures 22%: inventory reservation timeout"),
            alert("09:11:05", "orders-db", "major", "primary failover pg-orders-a -> pg-orders-b"),
            alert("09:11:50", "orders-db", "major", "replica lag 95 s on pg-orders-c"),
            alert("09:12:00", "inventory", "major", "stale stock reads; reservation conflicts 14%"),
            alert("09:12:40", "payments", "minor", "capture retries up 3x (upstream order not found)"),
            alert("09:10:00", "sso-bridge", "warning", "SAML signing certificate rotates in 7 days"),
            alert("09:12:30", "api-gateway", "major", "5xx 6% on /v1/orders"),
        ],
        "changes": [
            {"time": "08:55:00", "service": "pricing", "change": "deploy pricing v8.4.0 (discount rounding)"},
        ],
        "draft": "We are experiencing a major database outage and all customer order data may be lost. Engineers expect a full fix within 5 minutes.",
        "expected": {
            "cluster": [0, 1, 2, 3, 4, 6],
            "kind": "infrastructure_failure",
            "root": "orders-db",
            "publish": False,
        },
    },
    {
        "id": "credential-stuffing",
        "title": "계정 대입 공격: 방화벽, 인증, 게이트웨이가 함께 울리고 무관한 지연이 섞임",
        "alerts": [
            alert("02:20:10", "api-gateway", "major", "429 responses 40x baseline on POST /v1/login"),
            alert("02:19:40", "auth-service", "critical", "failed logins 31k/min (baseline 400), 9k distinct source IPs"),
            alert("02:20:30", "waf", "major", "bot score rule 2203 matches up 50x, residential proxy ASN"),
            alert("02:21:00", "session-store", "minor", "write latency p99 40 ms (baseline 8 ms)"),
            alert("02:18:00", "kafka", "warning", "consumer group analytics-etl lag 2 h (nightly batch)"),
            alert("02:21:20", "auth-service", "major", "successful logins from new devices 6x baseline"),
        ],
        "changes": [],
        "draft": "Our password database was breached tonight. Attackers used 9,000 IPs from residential proxies; we blocked rule 2203 on our WAF.",
        "expected": {
            "cluster": [0, 1, 2, 3, 5],
            "kind": "security",
            "root": "auth-service",
            "publish": False,
        },
    },
]


def run_case(chain: Chain, case: dict) -> None:
    alerts, expected = case["alerts"], case["expected"]
    order = {"critical": 0, "major": 1, "minor": 2, "warning": 3}
    # 기준 알림은 코드가 정한다. 계산할 수 있는 것은 judge에 묻지 않는다.
    primary = min(range(len(alerts)), key=lambda i: (order[alerts[i]["severity"]], alerts[i]["time"]))
    all_services = {svc: desc for svcs in SERVICES.values() for svc, desc in svcs.items()}

    # step 1: 묶기, 분류, 영향 + 비교용 한 번에 고르기
    q1 = {
        f"same_{i}": noul(
            {
                "alert": f"alerts[{i}]",
                "primary": f"alerts[{primary}]",
                "question": "Is `alert` part of the same customer-facing incident as `primary`, rather than a coincidental unrelated issue?",
            }
        )
        for i in range(len(alerts))
        if i != primary
    }
    q1["kind"] = choice("What kind of incident is the one around `primary_alert`?", KINDS)
    q1["impact"] = score(
        "How severe is the customer impact of the incident around `primary_alert`?",
        [
            "No customer impact",
            "A few customers see degraded performance",
            "Some customers fail a core action",
            "Many customers fail a core action",
            "Most customers cannot use the product",
        ],
    )
    q1["root_flat"] = choice(
        "Which service is the root cause of the incident around `primary_alert`?",
        all_services,
    )
    state1 = {"alerts": alerts, "recent_changes": case["changes"], "primary_alert": alerts[primary]}
    call1, a1 = chain.ask(state1, q1, trigger="alert_storm")

    cluster = sorted([primary] + [i for i in range(len(alerts)) if i != primary and a1[f"same_{i}"]["noul"] >= SAME_INCIDENT])
    chain.decide(
        "cluster",
        single_shot=list(range(len(alerts))),
        applied=cluster,
        applied_step=1,
        expected=expected["cluster"],
        note=f"{len(alerts)}개 중 {len(cluster)}개를 같은 장애로 묶음 (1단 기준선 = 시간 창 안의 모든 알림)",
    )
    chain.decide("kind", single_shot=a1["kind"]["choice"], applied=a1["kind"]["choice"], applied_step=1, expected=expected["kind"])

    # step 2: 묶인 알림만 남겨 방해 요소를 뺀 state로 도메인을 고른다.
    focused = {
        "alerts": [alerts[i] for i in cluster],
        "recent_changes": case["changes"],
        "inferred": {"kind": a1["kind"]["choice"], "impact_level": round(a1["impact"]["score"], 1)},
    }
    q2 = {"domain": choice("Which part of the system contains the root cause of the incident in `alerts`?", DOMAINS)}
    call2, a2 = chain.ask(focused, q2, trigger="cluster_ready", parent=call1)
    frontier = sorted(a2["domain"]["probabilities"].items(), key=lambda kv: -kv[1])[:BEAM]

    # step 3: 경로 3개의 서비스를 병렬로 묻는다.
    q3 = {
        f"svc_in_{domain}": choice(
            {"domain": domain, "question": "Within `domain`, which service is the root cause of the incident in `alerts`?"},
            SERVICES[domain],
        )
        for domain, _ in frontier
    }
    _, a3 = chain.ask(focused, q3, trigger=f"beam_k{BEAM}", parent=call2)
    paths = []
    for domain, p_domain in frontier:
        dist = a3[f"svc_in_{domain}"]["probabilities"]
        for svc, p_svc in dist.items():
            paths.append((path_score([p_domain, p_svc]), domain, svc))
    paths.sort(reverse=True)
    best, runner_up = paths[0], paths[1]
    greedy = a3[f"svc_in_{frontier[0][0]}"]["choice"]
    chain.decide(
        "root",
        single_shot=a1["root_flat"]["choice"],
        applied=best[2],
        applied_step=3,
        expected=expected["root"],
        note=f"beam {best[1]}/{best[2]} score={best[0]:.2f}, 분리도 {best[0] / max(runner_up[0], 1e-9):.1f}x, greedy={greedy}",
    )

    # step 4: 공지 초안 검증. 앞 판단은 inferred로 넣어 사실과 구분한다.
    checks = {
        "overstates_impact": "Does `draft` claim more harm than `alerts` and `inferred.impact_level` support?",
        "names_unconfirmed_cause": "Does `draft` state a cause, such as a breach or data loss, that `alerts` do not confirm?",
        "leaks_internal_detail": "Does `draft` reveal internal detail customers should not see, such as rule ids, IP counts, hostnames, or flag names?",
        "promises_unverifiable_eta": "Does `draft` promise a fix time that nothing in `alerts` supports?",
    }
    state4 = {
        "observed": {"alerts": [alerts[i] for i in cluster], "draft": case["draft"]},
        "inferred": {"kind": a1["kind"]["choice"], "root_service": best[2], "impact_level": round(a1["impact"]["score"], 1)},
    }
    q4 = {qid: noul(text, true="The draft must be held for the incident commander.", false="Fine to publish.") for qid, text in checks.items()}
    _, a4 = chain.ask(state4, q4, trigger="draft_ready", parent=call1)
    hold, worst, value = max_gate(a4, list(checks), HOLD_AT)
    chain.decide(
        "publish",
        single_shot=True,
        applied=not hold,
        applied_step=4,
        expected=expected["publish"],
        note=f"max {worst}={value:.2f} -> {'지휘자에게 보류' if hold else '게시'}",
    )


if __name__ == "__main__":
    run_demo("incident_commander", CASES, run_case, max_steps=4)
