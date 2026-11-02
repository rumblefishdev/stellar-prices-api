#!/usr/bin/env python3
"""0331 pre-run report: every dollar token (USD/USDT/USDC, any issuer) traded
against XLM before 2021-02-01, its implied XLM/USD at par, and how it compares
with the day's other dollar markets and with Binance XLM/USDT.

Resumable: every fetch is cached under OUT. Usage: KEY=... collect.py OUT
"""
import csv, json, os, ssl, statistics, sys, time, urllib.error, urllib.request

CTX = ssl.create_default_context(cafile="/etc/ssl/cert.pem")

OUT = sys.argv[1]
API = "https://prices-api.sorobanscan.rumblefish.dev/v1"
KEY = os.environ["KEY"]
UA = {"User-Agent": "stellar-prices-skill/1"}
CODES = ["USD", "USDT", "USDC"]
CANON = {
    ("USDC", "GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN"),
    ("USDT", "GCQTGZQQ5G4PTM2GL7CDIFKUBIPEC52BROAQIAPW53XBRJVN6ZJVTG6V"),
}
START, END = "2015-11-01", "2021-01-31"
os.makedirs(f"{OUT}/daily", exist_ok=True)


def get(url, headers, pace=0.0):
    for i in range(6):
        try:
            with urllib.request.urlopen(urllib.request.Request(url, headers=headers), timeout=90, context=CTX) as r:
                data = json.load(r)
            time.sleep(pace)
            return data
        except urllib.error.HTTPError as e:
            time.sleep(max(pace, 1))
            if e.code == 404:
                return None
            if e.code == 400:
                return {"_err": 400, "body": e.read().decode()[:300]}
            if e.code in (429, 500, 502, 503, 504):
                time.sleep(2 * (i + 1))
                continue
            raise
        except Exception:
            time.sleep(3 * (i + 1))
    return {"_err": "retries"}


def api(path):
    return get(API + path, {**UA, "x-api-key": KEY}, pace=1.05)


def cached(path, fn):
    if os.path.exists(path):
        with open(path) as f:
            return json.load(f)
    data = fn()
    with open(path, "w") as f:
        json.dump(data, f)
    return data


# 1. Every issuer of a dollar code, from Horizon.
def enumerate_assets():
    out = []
    for code in CODES:
        url = f"https://horizon.stellar.org/assets?asset_code={code}&limit=200"
        while url:
            page = get(url, UA)
            recs = page["_embedded"]["records"]
            out += [{"code": r["asset_code"], "issuer": r["asset_issuer"],
                     "accounts": r.get("accounts", {}).get("authorized")} for r in recs]
            url = page["_links"]["next"]["href"] if len(recs) == 200 else None
    return out


assets = cached(f"{OUT}/assets.json", enumerate_assets)
print(f"assets: {len(assets)}", flush=True)

# 2. Monthly scan: which of them traded against XLM (token = base) before 2021-02.
mpath = f"{OUT}/monthly.jsonl"
done = set()
if os.path.exists(mpath):
    with open(mpath) as f:
        done = {json.loads(l)["id"] for l in f}
with open(mpath, "a") as f:
    for i, a in enumerate(assets):
        aid = f"{a['code']}:{a['issuer']}"
        if aid in done or (a["code"], a["issuer"]) in CANON:
            continue
        r = api(f"/assets/{aid}/ohlcv?start={START}&end={END}&granularity=1M&base_currency=XLM")
        months = [] if not r or "data" not in r else [
            {"m": d["timestamp"][:7], "vol": d["volume_base"], "n": d["trade_count"]} for d in r["data"]]
        f.write(json.dumps({"id": aid, "months": months, "err": (r or {}).get("_err")}) + "\n")
        f.flush()
        if i % 50 == 0:
            print(f"monthly {i}/{len(assets)}", flush=True)

with open(mpath) as f:
    monthly = [json.loads(l) for l in f]
traded = [m for m in monthly if m["months"]]
print(f"traded vs XLM before 2021-02: {len(traded)}", flush=True)

# 3. Daily series for the traded ones.
daily = {}
for m in traded:
    aid = m["id"]
    p = f"{OUT}/daily/{aid.replace(':', '_')}.json"
    daily[aid] = cached(p, lambda: api(f"/assets/{aid}/ohlcv?start={START}&end={END}&granularity=1d&base_currency=XLM"))

# 4. External reference: Binance XLM/USDT daily (from 2018-05-31).
def binance():
    rows, t = [], 1514764800000
    while t < 1612137600000:
        k = get(f"https://api.binance.com/api/v3/klines?symbol=XLMUSDT&interval=1d&startTime={t}&limit=1000", UA)
        if not k:
            break
        rows += k
        t = k[-1][0] + 86400000
    return {time.strftime("%Y-%m-%d", time.gmtime(r[0] / 1000)): float(r[7]) / float(r[5])
            for r in rows if float(r[5]) > 0}

bn = cached(f"{OUT}/binance.json", binance)

# 5. XLM's own daily series (XLM as base = the canonical XLM/USDT market before 2021).
xlm = cached(f"{OUT}/xlm.json", lambda: api(f"/assets/native/ohlcv?start={START}&end={END}&granularity=1d"))

# 6. Analysis.
per_day = {}  # date -> list of (aid, implied_xlm_usd, vol_usd)
for aid, r in daily.items():
    for d in (r or {}).get("data", []):
        px = d.get("pf_vwap") or d.get("vwap")
        if not px or float(px) <= 0:
            continue
        per_day.setdefault(d["timestamp"][:10], []).append((aid, 1 / float(px), float(d["volume_base"])))


def wmedian(pairs):
    pairs = sorted(pairs)
    tot, acc = sum(w for _, w in pairs), 0.0
    for v, w in pairs:
        acc += w
        if acc >= tot / 2:
            return v
    return None


day_rows, tok = [], {}
for day in sorted(per_day):
    mk = per_day[day]
    vol = sum(v for _, _, v in mk)
    med = wmedian([(x, v) for _, x, v in mk if v > 0]) if vol > 0 else statistics.median(x for _, x, _ in mk)
    ext = bn.get(day)
    day_rows.append({"date": day, "markets": len(mk), "vol_usd_at_par": round(vol, 2),
                     "ref_wmedian": round(med, 6), "binance_vwap": round(ext, 6) if ext else "",
                     "ref_vs_binance_pct": round((med / ext - 1) * 100, 2) if ext else ""})
    for aid, x, v in mk:
        t = tok.setdefault(aid, {"days": 0, "vol": 0.0, "first": day, "last": day, "dev_med": [], "dev_bn": []})
        t["days"] += 1; t["vol"] += v; t["last"] = day
        t["dev_med"].append(x / med - 1)
        if ext:
            t["dev_bn"].append(x / ext - 1)

tok_rows = []
for aid, t in sorted(tok.items(), key=lambda kv: -kv[1]["vol"]):
    dm = [abs(x) for x in t["dev_med"]]
    db = [abs(x) for x in t["dev_bn"]]
    tok_rows.append({"asset": aid, "days": t["days"], "vol_usd_at_par": round(t["vol"], 2),
                     "first": t["first"], "last": t["last"],
                     "median_abs_dev_vs_day_pct": round(statistics.median(dm) * 100, 2),
                     "days_dev_gt_10pct": sum(x > 0.10 for x in dm), "days_dev_gt_25pct": sum(x > 0.25 for x in dm),
                     "median_abs_dev_vs_binance_pct": round(statistics.median(db) * 100, 2) if db else "",
                     "binance_days": len(db)})

for name, rows in (("tokens.csv", tok_rows), ("daily.csv", day_rows)):
    with open(f"{OUT}/{name}", "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=list(rows[0]))
        w.writeheader(); w.writerows(rows)

xlm_days = {}
for d in (xlm or {}).get("data", []):
    y = d["timestamp"][:4]
    xlm_days.setdefault(y, [0, 0.0]); xlm_days[y][0] += 1; xlm_days[y][1] += float(d["volume_base"])
years = {}
for r in day_rows:
    y = r["date"][:4]
    years.setdefault(y, {"days_with_ref": 0, "vol_usd": 0.0})
    years[y]["days_with_ref"] += 1; years[y]["vol_usd"] += r["vol_usd_at_par"]
summary = {"assets_enumerated": len(assets), "scanned": len(monthly),
           "errors": sum(1 for m in monthly if m["err"]), "traded_vs_xlm": len(traded),
           "days_with_ref": len(day_rows), "years": years,
           "xlm_own_series": {y: {"days": v[0], "vol_xlm": round(v[1], 2)} for y, v in xlm_days.items()},
           "binance_days": len(bn)}
with open(f"{OUT}/summary.json", "w") as f:
    json.dump(summary, f, indent=1)
print(json.dumps(summary, indent=1))
