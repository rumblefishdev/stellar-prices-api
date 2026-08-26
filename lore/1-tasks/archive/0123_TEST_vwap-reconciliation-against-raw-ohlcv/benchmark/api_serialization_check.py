#!/usr/bin/env python3
"""0123 final AC: no precision loss between CH and the public API JSON.
Compares the API response (Decimal strings) against the current_prices row
captured at the SAME updated_at tick, as exact Decimal values, and asserts
the JSON really carries strings (the §3.3 design point)."""
import csv, json, sys
from decimal import Decimal as D

api = json.load(open("api-xlm.json"))
ch = next(csv.DictReader(open("ch-xlm.csv")))

api_tick = api["updated_at"].replace("T", " ").rstrip("Z")
assert api_tick == ch["updated_at"], f"tick mismatch: {api_tick} vs {ch['updated_at']}"

fails = 0
def check(name, api_val, ch_val, str_required=True):
    global fails
    ok_type = isinstance(api_val, str) or not str_required
    ok_val = D(api_val) == D(ch_val)
    ok = ok_type and ok_val
    if not ok: fails += 1
    kind = "str" if isinstance(api_val, str) else type(api_val).__name__
    print(f"  {'OK' if ok else 'MISMATCH':8s} {name:28s} api={api_val} ({kind})  ch={ch_val}")

print(f"pinned tick: {api_tick}")
for f_api, f_ch in [("price_usd","price_usd"), ("price_xlm","price_xlm"),
                    ("vwap_24h","vwap_24h"), ("volume_24h_usd","volume_24h_usd"),
                    ("change_24h_pct","change_24h_pct")]:
    check(f_api, api[f_api], ch[f_ch])
ch_src = json.loads(ch["sources"])
assert set(api["sources"]) == set(ch_src), f"source keys differ: {sorted(api['sources'])} vs {sorted(ch_src)}"
for s in sorted(api["sources"]):
    check(f"sources[{s}].price", api["sources"][s]["price"], ch_src[s]["price"])
    check(f"sources[{s}].volume_24h", api["sources"][s]["volume_24h"], ch_src[s]["volume_24h"])
print("RESULT:", "NO PRECISION LOSS — all Decimal fields string-serialised and value-exact" if fails == 0 else f"{fails} FAILED")
sys.exit(1 if fails else 0)
