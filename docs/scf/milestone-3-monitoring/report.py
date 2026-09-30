#!/usr/bin/env python3
"""Figures for the Tranche 3 AC 9 report (task 0296), from export.sh's output.

The definitions are in ../milestone-3-monitoring-report.md §2; this only
computes them. Prints Markdown. `python3 report.py --self-test` checks the
uptime arithmetic.
"""
import json
import sys
from collections import defaultdict
from datetime import datetime, timedelta, timezone
from pathlib import Path

START = datetime(2026, 9, 23, 7, 40, tzinfo=timezone.utc)
MINUTES = 7 * 24 * 60
INTERVAL = 5  # minutes per uptime interval (§2)
LAG_OK_S = 120  # threshold of prices-production-ledger-processor-lag
FRESH_OK_S = 900  # threshold of prices-production-current-prices-freshness


def load(path):
    """{id: {timestamp: value}}; get-metric-data pages repeat an id, merge them."""
    out = defaultdict(dict)
    for r in json.loads(Path(path).read_text())["MetricDataResults"]:
        for t, v in zip(r["Timestamps"], r["Values"]):
            out[r["Id"]][datetime.fromisoformat(t).astimezone(timezone.utc)] = v
    return out


def by_minute(points):
    return {int((t - START).total_seconds() // 60): v for t, v in points.items()}


def uptime(count, e5, minutes=MINUTES, interval=INTERVAL):
    """100 % minus the mean 5XX rate of the intervals; no requests = 0 % errors."""
    rates = []
    for s in range(0, minutes, interval):
        n = sum(count.get(i, 0) for i in range(s, s + interval))
        f = sum(e5.get(i, 0) for i in range(s, s + interval))
        rates.append(f / n if n else 0.0)
    return 100 * (1 - sum(rates) / len(rates))


def one(agg, id_):
    v = list(agg.get(id_, {}).values())
    return v[0] if v else None


def fmt(v, nd=1):
    return "no data" if v is None else f"{v:,.{nd}f}"


def report(d):
    m, w, day = load(d / "minute.json"), load(d / "window.json"), load(d / "daily.json")
    count, e4, e5 = (by_minute(m[k]) for k in ("gw_count", "gw_4xx", "gw_5xx"))
    lag = by_minute(m["ingest_lag"])
    fresh = load(d / "rollup.json")["fresh_current"]

    n, n4, n5 = sum(count.values()), sum(e4.values()), sum(e5.values())
    print("## Figures\n")
    print(f"Window {START:%Y-%m-%d %H:%M} UTC + 7 days, {MINUTES:,} minutes.\n")
    print("| figure | value |\n|---|---|")
    print(f"| uptime (§2, {INTERVAL}-minute intervals) | {uptime(count, e5):.3f} % |")
    print(f"| minutes with at least one 5XX | {sum(1 for v in e5.values() if v)} |")
    print(f"| minutes with any request | {sum(1 for v in count.values() if v):,} of {MINUTES:,} |")
    print(f"| requests | {n:,.0f} |")
    if n:
        print(f"| 5XX rate | {100 * n5 / n:.3f} % ({n5:,.0f}) |")
        print(f"| 4XX rate (client errors, not downtime) | {100 * n4 / n:.2f} % ({n4:,.0f}) |")
    for p in ("p50", "p95", "p99"):
        print(f"| latency {p}, gateway (ms) | {fmt(one(w, 'lat_' + p))} |")
    for p in ("p50", "p95", "p99"):
        print(f"| integration latency {p}, Lambda path (ms) | {fmt(one(w, 'ilat_' + p))} |")
    hit, miss = one(w, "cache_hit") or 0, one(w, "cache_miss") or 0
    print(f"| cache hit ratio | {100 * hit / (hit + miss):.1f} % |" if hit + miss else "| cache hit ratio | no data |")
    for k, label in (("api_err", "api-handler errors"), ("api_thr", "api-handler throttles"),
                     ("lp_err", "ledger-processor errors"), ("portal_closed", "portal closed at cold start"),
                     ("portal_failed", "portal source loads failed")):
        print(f"| {label} | {fmt(one(w, k), 0)} |")
    ok = sum(1 for v in lag.values() if v <= LAG_OK_S)
    print(f"| ingest lag ≤ {LAG_OK_S} s | {100 * ok / len(lag):.2f} % of {len(lag):,} minutes with data; max {max(lag.values()):,.0f} s |"
          if lag else "| ingest lag | no data |")
    okf = sum(1 for v in fresh.values() if v <= FRESH_OK_S)
    print(f"| current_prices lag ≤ {FRESH_OK_S} s | {100 * okf / len(fresh):.2f} % of {len(fresh)} probes; max {max(fresh.values()):,.0f} s |"
          if fresh else "| current_prices freshness | no data |")

    print("\n## Per day (09:40 → 09:40 CEST)\n")
    print("| day from | requests | 5XX | 4XX | p95 (ms) | p99 (ms) |\n|---|---|---|---|---|---|")
    for t in sorted(day["gw_count"]):
        g = lambda k: day.get(k, {}).get(t)
        print(f"| {t + timedelta(hours=2):%m-%d %H:%M} | {fmt(g('gw_count'), 0)} | {fmt(g('gw_5xx'), 0)} "
              f"| {fmt(g('gw_4xx'), 0)} | {fmt(g('lat_p95'))} | {fmt(g('lat_p99'))} |")

    print("\n## Alarm transitions in the window (prices-production-*)\n")
    items = json.loads((d / "alarm-history.json").read_text())["AlarmHistoryItems"]
    rows = sorted((i["Timestamp"], i["AlarmName"], i["HistorySummary"]) for i in items
                  if i["AlarmName"].startswith("prices-production-"))
    print("| when | alarm | transition |\n|---|---|---|")
    for t, name, s in rows:
        print(f"| {datetime.fromisoformat(t).astimezone(timezone(timedelta(hours=2))):%m-%d %H:%M} CEST "
              f"| {name.removeprefix('prices-production-')} | {s.removeprefix('Alarm updated from ')} |")

    status, tip = d / "backfill-status.json", d / "network-tip.json"
    if status.exists() and tip.exists():
        s = json.loads(status.read_text())
        net = json.loads(tip.read_text())["_embedded"]["records"][0]["sequence"]
        exported = (d / "exported-at.txt").read_text().strip()
        print(f"\n## Point samples at export ({exported})\n")
        print(f"- `sdex.earliest_data_available`: {s.get('sdex', {}).get('earliest_data_available')}")
        print(f"- `realtime_tip_ledger`: {s['realtime_tip_ledger']} against the network's {net} "
              f"({net - s['realtime_tip_ledger']} ledgers behind)")


def self_test():
    ten = {i: 10 for i in range(10)}
    assert uptime(ten, {i: 10 for i in range(5)}, minutes=10) == 50.0  # one of two intervals fully failing
    assert uptime(ten, {0: 10}, minutes=10) == 90.0  # 10 of 50 in one interval = 20 %, mean 10 %
    assert uptime({}, {}, minutes=10) == 100.0  # no requests = no errors
    assert uptime({0: 100, 1: 100}, {0: 1}, minutes=5) == 99.5  # 1 of 200 in one interval
    assert uptime(ten, {}, minutes=10) == 100.0
    print("self-test ok")


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        self_test()
    else:
        report(Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).parent / "data")
