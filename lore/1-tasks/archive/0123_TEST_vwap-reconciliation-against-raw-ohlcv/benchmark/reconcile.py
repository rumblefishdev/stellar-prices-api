#!/usr/bin/env python3
"""0123 — independent recompute of current_prices from raw price_ohlcv_1m rows.

Implements the §5.5 + current.sql CONTRACT from spec, deliberately not a
translation of the CTE SQL:
  per-source:  price = newest PRICED close (close_usd > 0); volume = plain sum;
               live = newest candle (priced or not) within 2h of T
  population:  sources with a priced close in the window
  guard:       drop stale sources ONLY if a live one survives (conditional)
  mask:        unweighted median, arms at >=3 kept, band +/-20%
  vwap:        sum(p*v)/sum(v) over survivors, Float64 like the MV
  price_usd:   newest priced close across ALL sources (never masked)
  volume_24h:  plain sum over ALL rows (never masked, never filtered)

Ties on the newest-priced timestamp are collected, not resolved: the MV's
argMaxIf tie-break is non-contractual, so published must be IN the tie set.
"""
import csv, json, sys
from decimal import Decimal
from datetime import datetime, timedelta
from itertools import product
from statistics import median

D = Decimal
BAND = 0.20          # OUTLIER_PCT
MASK_MIN = 3         # mask arms at >= 3 kept sources
LIVE_H = 2           # liveness horizon, hours
VWAP_RTOL = 1e-9     # Float64 carries ~15-16 sig digits; MV computes vwap in
                     # Float64 then casts Decimal(38,14) -> 1e-9 rel is generous
NAMES = {4: "XLM", 5: "AQUA", 70: "SCOP", 108: "BTC", 430: "EURC", 741: "USDCAllow"}

def parse_ts(s): return datetime.strptime(s, "%Y-%m-%d %H:%M:%S")

def newest_priced(rows):
    """(tie_set_of_close_usd, ts) among rows with close_usd > 0."""
    priced = [r for r in rows if r["close_usd"] > 0]
    if not priced: return set(), None
    tmax = max(r["ts"] for r in priced)
    return {r["close_usd"] for r in priced if r["ts"] == tmax}, tmax

def load(path_current, path_raw):
    with open(path_current) as f:
        pub = {int(r["asset_id"]): r for r in csv.DictReader(f)}
    raw = {}
    with open(path_raw) as f:
        for r in csv.DictReader(f):
            row = {"ts": parse_ts(r["timestamp"]), "src": r["source"],
                   "close_usd": D(r["close_usd"]), "vol": D(r["volume_quote_usd"])}
            raw.setdefault(int(r["asset_id"]), []).append(row)
    return pub, raw

def recompute(rows, live_cutoff):
    out = {"volume_24h": sum(r["vol"] for r in rows)}
    out["price_usd_set"], out["price_usd_ts"] = newest_priced(rows)
    srcs = {}
    for r in rows: srcs.setdefault(r["src"], []).append(r)
    per = {}
    for s, sr in srcs.items():
        pset, pts = newest_priced(sr)
        per[s] = {"price_set": pset, "price_ts": pts,
                  "vol": sum(r["vol"] for r in sr),
                  "live": max(r["ts"] for r in sr) >= live_cutoff}
    out["per_source"] = per
    population = {s for s, v in per.items() if v["price_set"]}
    out["excl_population"] = set(per) - population          # rows, never priced
    has_live = any(per[s]["live"] for s in population)
    kept = {s for s in population if (not has_live) or per[s]["live"]}
    out["excl_guard"] = population - kept
    out["has_live"] = has_live
    if len(kept) >= MASK_MIN:
        prices = {s: float(next(iter(per[s]["price_set"]))) for s in kept}
        med = median(prices.values())
        out["median"], out["mask_armed"] = med, True
        surv = {s for s in kept if med > 0 and abs(prices[s] - med) / med <= BAND}
        out["deviations"] = {s: abs(prices[s] - med) / med for s in kept}
    else:
        out["mask_armed"], surv = False, kept
    out["excl_mask"] = kept - surv
    out["survivors"] = surv
    # argMaxIf tie-break across quote legs is NON-CONTRACTUAL: enumerate every
    # tie combination; the published vwap must match one of them.
    slist = sorted(surv)
    den = sum(float(per[s]["vol"]) for s in slist)
    out["vwap_combos"] = []
    for combo in product(*(sorted(per[s]["price_set"]) for s in slist)):
        num = sum(float(p) * float(per[s]["vol"]) for s, p in zip(slist, combo))
        out["vwap_combos"].append((num / den if den else 0.0, dict(zip(slist, combo))))
    return out

def verdict(ok): return "OK" if ok else "MISMATCH"

def main():
    pub, raw = load(sys.argv[1], sys.argv[2])
    ts = {r["updated_at"] for r in pub.values()}
    assert len(ts) == 1, f"mixed refresh ticks: {ts}"
    T = parse_ts(ts.pop()); cutoff = T - timedelta(hours=LIVE_H)
    print(f"pinned T = {T}  window = [{T - timedelta(hours=24)}, {T}]  live cutoff = {cutoff}\n")
    xlm_usd = D(pub[4]["price_usd"])
    failures = 0
    for aid in sorted(pub, key=lambda a: NAMES[a]):
        p, rows = pub[aid], raw.get(aid, [])
        rc = recompute(rows, cutoff)
        name = NAMES[aid]
        print(f"== {name} (asset_id {aid}) — {len(rows)} raw rows, "
              f"{len(rc['per_source'])} sources, has_live={rc['has_live']}, "
              f"mask_armed={rc['mask_armed']}")
        checks = []
        # price_usd: published must equal (or sit in the tie set of) newest priced close
        pp = D(p["price_usd"])
        tie = len(rc["price_usd_set"]) > 1
        checks.append(("price_usd", pp in rc["price_usd_set"],
            f"pub {pp} vs computed {sorted(rc['price_usd_set'])}"
            + (f"  [TIE x{len(rc['price_usd_set'])} @ {rc['price_usd_ts']}]" if tie else "")))
        # volume_24h_usd: plain Decimal sum, exact
        pv = D(p["volume_24h_usd"])
        checks.append(("volume_24h_usd", pv == rc["volume_24h"],
            f"pub {pv} vs computed {rc['volume_24h']} (delta {pv - rc['volume_24h']})"))
        # vwap: Float64 tolerance, best tie combination wins
        pw = float(D(p["vwap_24h"]))
        best = min(rc["vwap_combos"],
                   key=lambda c: abs(pw - c[0]) / pw if pw else abs(c[0]))
        dw = abs(pw - best[0]) / pw if pw else abs(best[0])
        ncombo = len(rc["vwap_combos"])
        checks.append(("vwap_24h", dw <= VWAP_RTOL,
            f"pub {p['vwap_24h']} vs computed {best[0]:.14f} (rel {dw:.2e}"
            + (f", best of {ncombo} tie combos)" if ncombo > 1 else ")")))
        # sources JSON: keys must equal survivors; values Decimal-equal
        pj = json.loads(p["sources"])
        checks.append(("sources keys", set(pj) == rc["survivors"],
            f"pub {sorted(pj)} vs computed {sorted(rc['survivors'])}"))
        for s in sorted(set(pj) & rc["survivors"]):
            ps, pv_ = D(pj[s]["price"]), D(pj[s]["volume_24h"])
            checks.append((f"sources[{s}].price", ps in rc["per_source"][s]["price_set"],
                f"pub {ps} vs {sorted(rc['per_source'][s]['price_set'])}"))
            checks.append((f"sources[{s}].volume", pv_ == rc["per_source"][s]["vol"],
                f"pub {pv_} vs {rc['per_source'][s]['vol']}"))
        # price_xlm: ratio through XLM's own price_usd; XLM itself must be exactly 1
        px = D(p["price_xlm"])
        if aid == 4:
            checks.append(("price_xlm", px == 1, f"pub {px} (must be exactly 1)"))
        elif rc["price_usd_set"]:
            exp = float(pp) / float(xlm_usd)
            dr = abs(float(px) - exp) / exp if exp else 0
            checks.append(("price_xlm", dr <= 1e-9, f"pub {px} vs {exp:.14f} (rel {dr:.2e})"))
        for fld, ok, detail in checks:
            if not ok: failures += 1
            print(f"   {verdict(ok):8s} {fld:22s} {detail}")
        # excluded-set attribution
        print(f"   excluded: population-filter={sorted(rc['excl_population'])} "
              f"guard={sorted(rc['excl_guard'])} mask={sorted(rc['excl_mask'])}")
        if rc["mask_armed"]:
            dev = {s: f"{v:.4%}" for s, v in rc["deviations"].items()}
            print(f"   mask: median={rc['median']:.10f} deviations={dev}")
        print()
    print(f"RESULT: {'ALL RECONCILED' if failures == 0 else f'{failures} check(s) failed'}")
    sys.exit(1 if failures else 0)

if __name__ == "__main__":
    main()
