#!/usr/bin/env python3
"""Task 0286 phase 3 — re-ingest the candle history, oldest month first.

Automates the month loop of docs/runbooks/0286-reingest-history.md and STOPS at
every gate that runbook names. Standard library only.

  plan       month -> ledger range, read from the Stellar history archive
  preflight  the runbook's preconditions, plus 0290's pool-registry gate
  run        the month loop (resumable; one month = twelve steps)
  status     the dashboard, once (use under `watch -n5` in a second pane)
  amm-done   record an events-backfill run someone did on the host (--amm wait)
             — the default, --amm ssh, runs events-backfill on the CH host (--ssh)
  finish     1w + 1M rebuild and the acceptance reads, after the last month
  rollback   put one month back from its snapshot (REPLACE PARTITION, SQL only)
  release    drop one finished month's snapshot, to give the disk back

Identities (task 0276 — no SSH, no `default` password). All three bundles live on
the campaign machine, fishuser-hero:~/prices-mtls/ :
  admin   prices-admin-production -> prices_admin  snapshot tables, DROP PARTITION, TRUNCATE
  writer  prices_writer                            marker DELETE, pre-roll INSERT
  reader  prices-admin-production -> prices_admin  every check

The admin cert is the reader too: it is uncapped, while dev_read carries a 30 s /
4 GB profile that cannot run the month-wide FINAL sums these checks need — and it
exhausted its 2 TiB/hour quota twice on 2026-09-22 alone. dev_shared is NOT used
(it holds DROP on every database, including BE's); task 0286's phase-3 section
records that decision.
"""

import argparse
import datetime as dt
import fcntl
import getpass
import gzip
import json
import os
import re
import shlex
import shutil
import socket
import ssl
import struct
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from decimal import Decimal
from pathlib import Path

UTC = dt.timezone.utc
SOROBAN_ACTIVATION_LEDGER = 50_457_424
ARCHIVE = "https://history.stellar.org/prd/core-live/core_live_001"
FINE_TIERS = ["15m", "1h", "4h", "1d"]
STEPS = [
    "gates", "snapshot", "before", "markers", "drop_1m", "sdex",
    "amm", "reconcile", "drop_coarse", "preroll", "verify", "done",
]
# Days written live between the durable-cursor break and the #313 deploy
# (2026-07-16 .. 2026-09-17 12:04 UTC): the SNAPSHOT is the damaged side.
LIVE_LOSS_MONTHS = range(202607, 202610)
# 0290: SushiSwap V3's first swap is at ledger 60,770,886 (2026-01).
SUSHI_FROM_MONTH = 202601
SUSHI_POOLS = 133
# 0300: Comet BLND/USDC's first swap is at ledger 51,500,460 (2024-05).
COMET_FROM_MONTH = 202405
COMET_POOLS = 1
# Measured on production 2026-09-23: every month 2024-05..2026-09 holds >= 135
# real Comet swaps, so an empty month up to here is a DEFECT. The pool is frozen
# and winding down after the 2026-08-25 exploit, so a LATER month may be empty
# for real — past this bound "no Comet volume" is only a FINDING.
COMET_MEASURED_THROUGH = 202609
# The first Comet swap (tx 237, 1000 USDC stroops -> 778,906 BLND stroops). A
# one-ledger events-backfill --dry-run here yields a `comet ticks: 1` line only
# from a binary that routes venue 'comet' — the proof the row gate cannot give.
COMET_PROBE_LEDGER = 51_500_460
# Set once the events-backfill binary has been proven to route Comet (per process).
COMET_BINARY_PROVEN = False
BAK = "reingest_0286_bak_"
# Waits between attempts when the campaign machine cannot reach ClickHouse at all
# (~8 min of sleeps, plus up to ~2 min of kernel connect timeout per attempt). On
# 2026-09-25 22:41 UTC one such blip — the server was serving BE and the Lambdas
# throughout — ended the run between two DROP PARTITIONs and cost 56 hours.
CONNECT_RETRY_DELAYS = (5, 15, 30, 60, 120, 240)


class Stop(Exception):
    """A gate said no. State is kept; fix the cause and `run` again."""


class ClickHouseDown(Exception):
    """ClickHouse is not answering behind the proxy. Exit 1: the wrapper resumes."""


def _connect_error_is_permanent(reason):
    """A connect failure no wait can fix: the server's certificate does not
    verify, the server rejected ours (a TLS alert: bad, expired, unknown CA,
    required), or the host name does not exist. A reset or EOF during the
    handshake, or EAI_AGAIN from a resolver that is offline, is still a blip."""
    if isinstance(reason, ssl.SSLCertVerificationError):
        return True
    if isinstance(reason, ssl.SSLError) and "ALERT" in str(reason).upper():
        return True
    return isinstance(reason, socket.gaierror) and reason.errno == socket.EAI_NONAME


# ---------------------------------------------------------------- ClickHouse

class CH:
    def __init__(self, a):
        self.url, self.db, self.dry = a.ch_url.rstrip("/"), a.database, a.dry_run
        self.ctx = {}
        if self.url.startswith("https"):
            for role, stem in (("admin", a.admin_cert), ("writer", a.writer_cert),
                               ("reader", a.reader_cert)):
                # The server presents a public (Let's Encrypt) certificate; --ca is the
                # internal CA that signs the CLIENT certs. Passing it as `cafile` alone
                # drops the system roots and fails CERTIFICATE_VERIFY_FAILED — curl hides
                # this by also reading its CApath, the Rust client by adding webpki roots.
                c = ssl.create_default_context()
                c.load_verify_locations(cafile=os.path.expanduser(a.ca))
                stem = os.path.expanduser(stem)
                c.load_cert_chain(stem + ".crt", stem + ".key")
                self.ctx[role] = c

    def _sql(self, sql):
        # The schema files and this script say `prices.`; a test run points
        # --database elsewhere.
        return sql if self.db == "prices" else re.sub(r"\bprices\.", self.db + ".", sql)

    def q(self, role, sql, params=None, settings=None, timeout=600):
        """One statement. `dev_read` is a readonly user: no settings in its URL."""
        qs = {f"param_{k}": v for k, v in (params or {}).items()}
        if role != "reader":
            qs.update(settings or {})
        url = self.url + "/" + ("?" + urllib.parse.urlencode(qs) if qs else "")
        for attempt, delay in enumerate(CONNECT_RETRY_DELAYS + (None,), 1):
            req = urllib.request.Request(url, data=self._sql(sql).encode(), method="POST")
            try:
                with urllib.request.urlopen(req, timeout=timeout, context=self.ctx.get(role)) as r:
                    return r.read().decode()
            except urllib.error.HTTPError as e:
                body = e.read().decode()[:600]
                if e.code not in (502, 503):
                    raise Stop(f"ClickHouse refused ({role}): {body}\n--- {sql[:300]}")
                # Caddy is up and ClickHouse behind it is not (a restart, an upstream
                # blip). Caddy also answers 502 when ClickHouse dies while running the
                # statement, so only a read is repeated here. A write ends the run with
                # exit 1 — not a STOP — and the wrapper's `run` resumes at the step.
                if role != "reader":
                    raise ClickHouseDown(f"ClickHouse unavailable behind the proxy ({role}): "
                                         f"HTTP {e.code} {body[:200]}\n--- {sql[:300]}")
                if delay is None:
                    raise ClickHouseDown(f"ClickHouse unavailable behind the proxy ({role}): "
                                         f"HTTP {e.code} after {attempt} attempts")
                log(f"ClickHouse unavailable ({role}): HTTP {e.code} — attempt {attempt} of "
                    f"{len(CONNECT_RETRY_DELAYS) + 1}, next in {delay} s")
                time.sleep(delay)
            except urllib.error.URLError as e:
                # urlopen raises URLError only while resolving, connecting or sending
                # the request, so ClickHouse never received a whole statement and a
                # write is as safe to repeat as a read. A failure after the request
                # went out (a timeout waiting for the answer, a dropped response)
                # surfaces as a bare OSError instead and is NOT retried: that
                # statement may have run, and only a `run` resume may repeat it.
                if _connect_error_is_permanent(e.reason):
                    raise Stop(f"ClickHouse unreachable ({role}), and retrying cannot help: "
                               f"{e.reason} — check the cert bundle, --ca and --ch-url")
                if delay is None:
                    raise
                log(f"ClickHouse unreachable ({role}): {e.reason} — attempt {attempt} of "
                    f"{len(CONNECT_RETRY_DELAYS) + 1}, next in {delay} s")
                time.sleep(delay)

    def rows(self, role, sql, **kw):
        out = self.q(role, sql.rstrip().rstrip(";") + " FORMAT TabSeparated", **kw)
        return [l.split("\t") for l in out.splitlines() if l]

    def one(self, role, sql, **kw):
        r = self.rows(role, sql, **kw)
        return r[0][0] if r else None

    def write(self, role, sql, **kw):
        if self.dry:
            log(f"[DRY {role}] {' '.join(self._sql(sql).split())[:200]}")
            return ""
        return self.q(role, sql, **kw)


# ------------------------------------------------- ledger <-> time (archive)
# Public Horizon only reaches back ~1 year (history_elder_ledger), so the
# runbook's "read closed_at from Horizon" cannot place a 2016 month. The history
# archive can: one checkpoint file holds 64 LedgerHeaderHistoryEntry records.

class Ledgers:
    def __init__(self, cache):
        self.cache = Path(cache)
        self.cache.mkdir(parents=True, exist_ok=True)
        self.mem = {}

    @staticmethod
    def _get(url):
        req = urllib.request.Request(url, headers={"User-Agent": "curl/8"})
        for attempt in range(5):
            try:
                return urllib.request.urlopen(req, timeout=60).read()
            except Exception:
                if attempt == 4:
                    raise
                time.sleep(2 ** attempt)

    def tip(self):
        return json.loads(self._get(ARCHIVE + "/.well-known/stellar-history.json"))["currentLedger"]

    def _checkpoint(self, cp):
        if cp in self.mem:
            return self.mem[cp]
        f = self.cache / f"{cp:08x}.json"
        if f.exists():
            out = {int(k): v for k, v in json.loads(f.read_text()).items()}
        else:
            h = f"{cp:08x}"
            data = gzip.decompress(self._get(
                f"{ARCHIVE}/ledger/{h[0:2]}/{h[2:4]}/{h[4:6]}/ledger-{h}.xdr.gz"))
            out, off = {}, 0
            while off < len(data):
                ln = struct.unpack(">I", data[off:off + 4])[0] & 0x7FFFFFFF
                rec, off = data[off + 4:off + 4 + ln], off + 4 + ln
                # hash32 | version4 | prevHash32 | StellarValue{txSetHash32, closeTime u64,
                # upgrades<>, ext} | txSetResultHash32 | bucketListHash32 | ledgerSeq
                close_time = struct.unpack(">Q", rec[100:108])[0]
                p = 108
                n = struct.unpack(">I", rec[p:p + 4])[0]
                p += 4
                for _ in range(n):
                    l = struct.unpack(">I", rec[p:p + 4])[0]
                    p += 4 + (l + 3) // 4 * 4
                ext = struct.unpack(">I", rec[p:p + 4])[0]
                p += 4 + (4 + 32 + 4 + 64 if ext == 1 else 0) + 64
                out[struct.unpack(">I", rec[p:p + 4])[0]] = close_time
            f.write_text(json.dumps(out))
        self.mem[cp] = out
        return out

    def closed_at(self, seq):
        return self._checkpoint(seq // 64 * 64 + 63)[seq]

    def first_at_or_after(self, ts, lo, hi):
        """Smallest ledger in [lo, hi] whose close time is >= ts (close times are monotonic)."""
        if self.closed_at(hi) < ts:
            raise Stop(f"the archive tip ({hi}) closed before {ts}: the month is not over yet")
        while lo < hi:
            mid = (lo + hi) // 2
            if self.closed_at(mid) >= ts:
                hi = mid
            else:
                lo = mid + 1
        return lo


def month_start(m):
    return dt.datetime(m // 100, m % 100, 1, tzinfo=UTC)


def next_month(m):
    return m + 89 if m % 100 == 12 else m + 1


def iso(ts):
    return dt.datetime.fromtimestamp(ts, UTC).strftime("%Y-%m-%d %H:%M:%S")


# --------------------------------------------------------------------- state

class State:
    def __init__(self, d, dry):
        self.dir, self.dry = Path(os.path.expanduser(d)), dry
        self.dir.mkdir(parents=True, exist_ok=True)
        self.f = self.dir / "state.json"
        self.d = json.loads(self.f.read_text()) if self.f.exists() else {"months": {}, "plan": {}}

    def save(self):
        if self.dry:
            return
        tmp = self.f.with_suffix(".tmp")
        tmp.write_text(json.dumps(self.d, indent=1))
        tmp.replace(self.f)

    def month(self, m):
        return self.d["months"].setdefault(str(m), {"step": 0})

    def lock(self):
        self._lock = open(self.dir / "lock", "w")
        try:
            fcntl.flock(self._lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError:
            raise Stop("another `run` holds the lock — one writer per source, always")


LOG = None


def log(msg, echo=True):
    line = f"{dt.datetime.now(UTC):%Y-%m-%d %H:%M:%S}Z {msg}"
    if LOG:
        with open(LOG, "a") as f:
            f.write(line + "\n")
    if echo and not sys.stdout.isatty():
        print(line, flush=True)


# ----------------------------------------------------------------- dashboard

DASH = {"month": None, "step": "", "since": time.time(), "tail": "", "notes": []}


def bar(done, total, width=40):
    fill = int(width * done / total) if total else 0
    return "[" + "#" * fill + "-" * (width - fill) + f"] {done}/{total}"


def render(st, months, clear=True):
    done = [m for m in months if st.month(m).get("step", 0) >= len(STEPS)]
    out = []
    out.append("0286 phase 3 — re-ingest under ADR 0287, oldest month first")
    out.append(f"months  {bar(len(done), len(months))}")
    durs = [st.month(m)["seconds"] for m in done if "seconds" in st.month(m)]
    if durs and len(done) < len(months):
        eta = sum(durs) / len(durs) * (len(months) - len(done))
        out.append(f"avg/month {sum(durs) / len(durs) / 60:.0f} min   remaining ~{eta / 3600:.1f} h"
                   f"   (excludes the re-enrichment that follows, runbook §7b)")
    if DASH["month"]:
        ms = st.month(DASH["month"])
        k = min(ms.get("step", 0), len(STEPS) - 1)
        out.append("")
        out.append(f"NOW  {DASH['month']}  ledgers {ms.get('start')}..{ms.get('end')}   "
                   f"step {k + 1}/{len(STEPS)} {STEPS[k]}   {DASH['step']}"
                   f"   ({(time.time() - DASH['since']) / 60:.1f} min)")
        if DASH["tail"]:
            out.append("     " + DASH["tail"][:150])
    out.append("")
    out.append(f"{'month':<7}{'ref':<4}{'sdex trades before -> after':<32}{'verdict':<9}"
               f"{'amm':<22}{'aligned':<9}{'no-order':<9}{'min':>5}")
    for m in [m for m in months if "result" in st.month(m)][-10:]:
        r = st.month(m)["result"]
        out.append(f"{m:<7}{r['ref']:<4}{r['sdex']:<32}{r['verdict']:<9}{r['amm']:<22}"
                   f"{r['aligned']:<9}{str(r['fallbacks']):<9}{st.month(m).get('seconds', 0) / 60:>5.0f}")
    for n in DASH["notes"][-6:]:
        out.append("  ! " + n)
    text = "\n".join(out)
    if clear and sys.stdout.isatty():
        sys.stdout.write("\x1b[2J\x1b[H" + text + "\n")
        sys.stdout.flush()
    return text


def note(msg):
    DASH["notes"].append(msg)
    log("NOTE " + msg)


# ------------------------------------------------------------------ the plan

def cmd_plan(a, ch, st):
    led = Ledgers(st.dir / "ledger-cache")
    tip = led.tip()
    first = a.from_month or int(ch.one("reader", """
        SELECT least(min(toYYYYMM(a)), min(toYYYYMM(b))) FROM
        (SELECT min(timestamp) AS a FROM prices.price_ohlcv_1m) AS x,
        (SELECT min(timestamp) AS b FROM prices.price_ohlcv_1d) AS y"""))
    today = dt.datetime.now(UTC)
    last_complete = (today.replace(day=1) - dt.timedelta(days=1))
    last = a.to_month or last_complete.year * 100 + last_complete.month
    if last > last_complete.year * 100 + last_complete.month:
        raise Stop("the current month belongs to the live processor (runbook §5): "
                   "a backfill range must never reach the live cursor")
    plan, lo, m = {}, 1, first
    while m <= last:
        ts = int(month_start(m).timestamp())
        start = led.first_at_or_after(ts, lo, tip)
        plan[m] = {"start": start}
        if m != first:
            plan[prev]["end"] = start - 1
        print(f"{m}  START {start:>9}  closed {iso(led.closed_at(start))}"
              f"   START-1 closed {iso(led.closed_at(start - 1)) if start > 1 else '-'}", flush=True)
        if start > 1 and led.closed_at(start - 1) >= ts:
            raise Stop(f"{m}: START-1 is not in the previous month")
        lo, prev, m = start, m, next_month(m)
    plan[prev]["end"] = led.first_at_or_after(int(month_start(m).timestamp()), lo, tip) - 1
    st.d["plan"] = {str(k): v for k, v in plan.items()}
    st.save()
    print(f"\n{len(plan)} months, {first}..{last}, archive tip {tip}. "
          "Every range starts on a month edge, which is a minute edge.")


def planned_months(a, st, ch):
    ms = sorted(int(m) for m in st.d["plan"])
    if not ms:
        raise Stop("no plan — run `plan` first")
    ms = [m for m in ms if (not a.from_month or m >= a.from_month)
          and (not a.to_month or m <= a.to_month)]
    return second_pass_months(a, st, ch, ms) if a.second_pass else ms


def second_pass_months(a, st, ch, planned):
    """0139 second pass: the planned months whose 1m rows the rekey did not copy."""
    if not a.to_month:
        raise Stop("--second-pass needs --to-month PAUSE_MONTH: later months are a first pass on the new ids")
    if not post0139_mode(ch):
        raise Stop("--second-pass before the 0139 swap would re-ingest into the old ids")
    listed = {int(m): int(c) + int(o) for m, c, o in ch.rows("reader", """SELECT month, colliding_rows,
        orphan_rows FROM prices.rekey_0139_reingest_months ORDER BY month""")}
    listed = {m: n for m, n in listed.items() if m <= a.to_month and (not a.from_month or m >= a.from_month)}
    missing = sorted(set(listed) - set(planned))
    if missing:
        raise Stop(f"rekey_0139_reingest_months lists {missing}, which the plan lacks: `plan` over them first")
    skipped = {m: n for m, n in listed.items() if n <= a.min_excluded_rows}
    lines = [f"second pass SKIPS {m}: {n} excluded 1m rows stay missing (--min-excluded-rows "
             f"{a.min_excluded_rows})" for m, n in sorted(skipped.items())]
    lines.append(f"second pass: {len(listed) - len(skipped)} of {len(listed)} listed months through {a.to_month}")
    for msg in lines:
        print(msg, flush=True)
        if a.command == "run":  # `status` under watch would repeat it every 5 s
            log(msg, echo=False)
    st.d["second_pass"] = {"to_month": a.to_month, "min_excluded_rows": a.min_excluded_rows,
                           "skipped": {str(m): n for m, n in sorted(skipped.items())}}
    # Not saved here: `status` holds no lock. `run` persists it with the month's first step.
    return [m for m in planned if m in listed and m not in skipped]


# ----------------------------------------------------------------- preflight

def cmd_preflight(a, ch, st, months=None):
    months = months or planned_months(a, st, ch)
    ok = []

    def gate(name, passed, detail):
        ok.append(passed)
        print(f"  {'ok  ' if passed else 'STOP'} {name}: {detail}", flush=True)

    v = ch.rows("reader", "SELECT version(), timezone(), serverTimezone()")[0]
    gate("server", v[1] == "UTC" and v[2] == "UTC", " / ".join(v))
    # The seven live tiers by name: after task 0139 every tier also has a
    # price_ohlcv_*__pre0139 copy with the same columns.
    tiers = ", ".join(f"'price_ohlcv_{t}'" for t in ["1m"] + FINE_TIERS + ["1w", "1M"])
    n = int(ch.one("reader", f"""SELECT count() FROM system.columns WHERE database = '{ch.db}'
        AND table IN ({tiers}) AND name IN ('pf_trade_count','pf_volume','pf_price_volume')"""))
    gate("phase 1 schema", n == 21, f"{n}/21 pf columns")
    old = []
    if post0139_mode(ch):
        print("  note 0139: the candle tables carry derived UInt64 asset ids (runbook §10)", flush=True)
        gate("0139 binaries", a.ack_0139_binaries,
             "sdex-backfill and events-backfill, here and on the CH host, are built from the 0139 merge "
             "or later — an older one writes UInt32 ids into UInt64 columns (--ack-0139-binaries)")
        n = int(ch.one("reader", f"""SELECT ifNull(sum(total_rows), 0) FROM system.tables
            WHERE database = '{ch.db}' AND name = 'asset_id_map_0139'"""))
        gate("0139 map", n > 0, f"prices.asset_id_map_0139 holds {n} rows — the reconcile reads the ids it copied")
        old = old_shape_baks(ch)
        gate("0139 snapshots", not old,
             f"{', '.join(old)} still on UInt32 ids: rename each to <name>_pre0139 (runbook §10b)"
             if old else "no old-shape reingest_0286_bak_* table")
    if old:
        gate("snapshot grants", False, "not probed: the probe would CREATE … IF NOT EXISTS next to an old-shape table")
    elif not ch.dry:
        try:  # partition 190001 does not exist: a no-op that still runs the access check
            ch.q("admin", f"CREATE TABLE IF NOT EXISTS prices.{BAK}price_ohlcv_1m AS prices.price_ohlcv_1m")
            ch.q("admin", f"ALTER TABLE prices.{BAK}price_ohlcv_1m ATTACH PARTITION 190001 FROM prices.price_ohlcv_1m")
            ch.q("admin", f"ALTER TABLE prices.price_ohlcv_1m REPLACE PARTITION 190001 FROM prices.{BAK}price_ohlcv_1m")
            gate("snapshot grants", True, "admin can CREATE a backup table, ATTACH PARTITION FROM into it, "
                                          "and REPLACE PARTITION back (the rollback)")
        except Stop as e:
            gate("snapshot grants", False, " ".join(str(e).split())[:220])
    gap = (Path(a.repo) / "packages/prices-clickhouse/schema/preroll-live-gap.sql")
    gate("checkout", gap.exists() and "pf_trade_count" in gap.read_text(),
         f"{gap} carries the post-0286 pre-roll")
    gate("phase 1 measured", a.ack_phase1_measured,
         "first-week measurement recorded on task 0286 (--ack-phase1-measured)")
    if any(m in LIVE_LOSS_MONTHS or m > LIVE_LOSS_MONTHS[-1] for m in months):
        gate("0285 reverse question", a.ack_0285,
             "live stored NONE of the unregistered pools' trades (0282 step 8, 904ba062), "
             "so DROP PARTITION deletes nothing events-backfill cannot rebuild (--ack-0285)")
    if any(m >= SUSHI_FROM_MONTH for m in months):
        n = int(ch.one("reader", f"SELECT count() FROM prices.pool_registry FINAL WHERE venue = '{a.sushi_source}'"))
        gate("0290 registry write", n >= SUSHI_POOLS,
             f"{n} {a.sushi_source} pools, need {SUSHI_POOLS}: phase 1 -> 0290 deploy (PR #324) -> "
             "--discover-pools WRITE -> phase 3. Earlier, the ~88.8k swaps are silently absent again")
    if any(m >= COMET_FROM_MONTH for m in months):
        n = int(ch.one("reader", f"SELECT count() FROM prices.pool_registry FINAL WHERE venue = '{a.comet_source}'"))
        gate("0300 registry write", n >= COMET_POOLS,
             f"{n} {a.comet_source} pools, need {COMET_POOLS}: 0300 deploy -> --discover-pools WRITE "
             "(any range; the static row is written regardless) -> phase 3. "
             "Earlier, the ~51.6k Comet swaps are silently absent again. The row alone does not route "
             "Comet — each month's gates step also proves the events-backfill binary does, before any DROP")
        if a.amm == "wait":
            gate("0300 events-backfill binary", a.ack_0300_binary,
                 "--amm wait: the host binary cannot be probed from here; check that it is built from 0300 "
                 "or later (a pre-0300 binary skips venue 'comet' silently), then pass --ack-0300-binary")
    free = int(ch.one("reader", "SELECT min(free_space) FROM system.disks"))
    gate("disk", free >= a.min_free_gb * 2 ** 30,
         f"{free / 2 ** 30:.0f} GiB free, floor {a.min_free_gb} — snapshots keep every dropped part alive")
    cur = ch.one("reader", "SELECT min(ledger) FROM prices.ingest_cursor FINAL")
    end = st.d["plan"][str(months[-1])]["end"]
    gate("live cursor", cur and int(cur) > end, f"live at {cur}, last planned END {end}")
    if not a.skip_aws_check:
        r = subprocess.run(["aws", "events", "describe-rule", "--name", a.cleanup_rule,
                            "--query", "State", "--output", "text"], capture_output=True, text=True)
        gate("cleanup (0200)", r.stdout.strip() == "DISABLED",
             f"{a.cleanup_rule} = {r.stdout.strip() or r.stderr.strip()[:120]}")
    exe = shlex.split(a.sdex_backfill)[0]
    gate("sdex-backfill", bool(shutil.which(exe) or os.path.exists(exe)),
         f"{exe}  (cargo build --release -p sdex-backfill --features aws-mtls)")
    if a.amm == "mtls" and any(st.d["plan"][str(m)]["end"] >= SOROBAN_ACTIVATION_LEDGER for m in months):
        exe = shlex.split(a.events_backfill)[0]
        gate("events-backfill", bool(shutil.which(exe) or os.path.exists(exe)), exe)
    if any(st.d["plan"][str(m)]["end"] >= SOROBAN_ACTIVATION_LEDGER for m in months):
        try:
            require_amm_path(a)
            gate("AMM path", True, f"--amm {a.amm}")
        except Stop as e:
            gate("AMM path", False, str(e))
    if any(st.d["plan"][str(m)]["end"] >= SOROBAN_ACTIVATION_LEDGER for m in months):
        print("  note AMM path: " +
             {"mtls": f"--amm mtls: {a.events_backfill} --transport {a.transport}, as the admin certificate "
                      "(cargo build --release -p events-backfill --features aws-mtls)",
              "stop": "--amm stop: the loop halts before the first Soroban-era month",
              "ssh": f"--amm ssh: events-backfill on {a.ssh} as `default`",
              "wait": "--amm wait: pauses for someone with host access, then `amm-done`"}[a.amm], flush=True)
    if not all(ok):
        raise Stop("preflight failed")


# ------------------------------------------------------------- month helpers

# 0139: rows under a formerly colliding id were not copied by the rekey, so a
# re-ingest brings them back under each identity's own id. Reconcile only the ids
# the rekey copied; the rest is reported, not judged.
COPIED_IDS = """(SELECT new_id FROM prices.asset_id_map_0139 GROUP BY new_id
    HAVING countIf(status IN ('mapped', 'sentinel')) > 0 AND countIf(status = 'colliding') = 0)"""
COLLIDING_IDS = "(SELECT new_id FROM prices.asset_id_map_0139 WHERE status = 'colliding')"
MAP_RESTRICT = f" AND asset_id IN {COPIED_IDS} AND quote_asset_id IN {COPIED_IDS}"

SUMS = """SELECT source, count(), sum(trade_count), sum(volume_base), sum(volume_quote)
FROM prices.price_ohlcv_{tier} FINAL WHERE toYYYYMM(timestamp) = {m}{restrict}
GROUP BY source ORDER BY source"""

OUTSIDE = f"""SELECT source,
    if(asset_id IN {COLLIDING_IDS} OR quote_asset_id IN {COLLIDING_IDS}, 'colliding', 'unmapped') AS class,
    sum(trade_count)
FROM prices.price_ohlcv_{{tier}} FINAL WHERE toYYYYMM(timestamp) = {{m}}
  AND NOT (asset_id IN {COPIED_IDS} AND quote_asset_id IN {COPIED_IDS})
GROUP BY source, class ORDER BY source, class"""


def sums(ch, tier, m, post=False):
    sql = SUMS.format(tier=tier, m=m, restrict=MAP_RESTRICT if post else "")
    return {r[0]: {"candles": int(r[1]), "trades": int(r[2]), "vb": r[3], "vq": r[4]}
            for r in ch.rows("reader", sql, timeout=3600)}


def outside(ch, tier, m):
    """Trades on ids the 0139 rekey did not copy, per `source class`."""
    return {f"{r[0]} {r[1]}": int(r[2])
            for r in ch.rows("reader", OUTSIDE.format(tier=tier, m=m), timeout=3600)}


def id_types(ch, table):
    """{column: type} of the id columns; {} when the table does not exist."""
    return dict(ch.rows("reader", f"""SELECT name, type FROM system.columns WHERE database = '{ch.db}'
        AND table = '{table}' AND name IN ('asset_id', 'quote_asset_id') ORDER BY name"""))


def post0139_mode(ch):
    """True once the 0139 swap put derived UInt64 ids into the candle tables."""
    return id_types(ch, "price_ohlcv_1m").get("asset_id") == "UInt64"


def old_shape_baks(ch):
    """Snapshot tables still in the pre-0139 id space under the orchestrator's own names."""
    return [r[0] for r in ch.rows("reader", f"""SELECT DISTINCT table FROM system.columns
        WHERE database = '{ch.db}' AND startsWith(table, '{BAK}') AND NOT endsWith(table, '_pre0139')
          AND name IN ('asset_id', 'quote_asset_id') AND type != 'UInt64' ORDER BY table""")]


def part_rows(ch, table, m):
    return int(ch.one("reader", f"""SELECT sum(rows) FROM system.parts WHERE database = '{ch.db}'
        AND table = '{table}' AND active AND partition = '{m}'"""))


PRE0139 = ("A snapshot taken before the 0139 window is in prices.{bak}_pre0139 (old ids, decoded by "
           "prices.asset_id_map_0139) and the pre-window rows in prices.{table}__pre0139; neither restores "
           "into the derived ids. Re-ingest the month instead (runbook 0286 §10, the second pass)")


def snapshot(ch, table, m):
    """Hardlink the partition into a backup table. SQL-only, so it restores over mTLS.

    The runbooks' `ATTACH PARTITION … FROM '/var/lib/clickhouse/shadow/…'` is a syntax
    error on 26.3.10.60 (FROM takes a table), and a FREEZE can only be restored by
    copying files on the host — which this operator has no SSH for.
    """
    bak = BAK + table
    ch.write("admin", f"CREATE TABLE IF NOT EXISTS prices.{bak} AS prices.{table}")
    have = {} if ch.dry else id_types(ch, bak)
    want = id_types(ch, table) if have else have
    if have != want:
        # CREATE … IF NOT EXISTS kept an old-shape table: never ATTACH across id widths.
        raise Stop(f"prices.{bak} holds {have.get('asset_id')} ids, prices.{table} {want.get('asset_id')}: "
                   f"rename it to {bak}_pre0139 (runbook 0286 §10b) before the next snapshot")
    if not ch.dry and part_rows(ch, bak, m):
        return "kept"  # never overwrite a pre-DROP snapshot with a post-DROP one
    src = part_rows(ch, table, m)
    if src == 0:
        return "empty"
    ch.write("admin", f"ALTER TABLE prices.{bak} ATTACH PARTITION {m} FROM prices.{table}")
    if not ch.dry and part_rows(ch, bak, m) != src:
        raise Stop(f"{bak} {m}: {part_rows(ch, bak, m)} rows captured of {src} — the snapshot is short")
    return f"{src} rows"


def statements(path, tiers):
    text = "\n".join(l for l in Path(path).read_text().splitlines() if not l.lstrip().startswith("--"))
    found = {}
    for s in (s.strip() for s in text.split(";")):
        t = re.match(r"INSERT INTO prices\.price_ohlcv_(\w+)", s)
        if t and t.group(1) in tiers:
            found[t.group(1)] = s
    missing = [t for t in tiers if t not in found]
    if missing:
        raise Stop(f"{path}: no INSERT for {missing}")
    return [found[t] for t in tiers]


def stream(cmd, env, logfile, stdin_text=None):
    """Run a child, tee its output, keep the dashboard alive. Returns (code, text)."""
    p = subprocess.Popen(cmd, env=env, stdin=subprocess.PIPE if stdin_text else subprocess.DEVNULL,
                         stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1)
    if stdin_text:
        p.stdin.write(stdin_text + "\n")
        p.stdin.close()
    lines, last = [], 0
    with open(logfile, "a") as f:
        for line in p.stdout:
            f.write(line)
            f.flush()
            lines.append(line)
            DASH["tail"] = re.sub(r"\x1b\[[0-9;]*m", "", line.strip())
            DASH["step"] = f"{sum('partition indexing complete' in l for l in lines)} partitions indexed"
            if time.time() - last > 5:
                last = time.time()
                RERENDER()
    return p.wait(), "".join(lines)


RERENDER = lambda: None


def require_transport_flag(a):
    """Stop unless the events-backfill binary accepts `--transport` (needed by --amm mtls).

    Shared by the Comet binary probe and the `amm` step, so a binary without the
    flag is reported as exactly that — never as a pre-0300 binary (PR #345 review).
    """
    try:
        probe = subprocess.run(shlex.split(a.events_backfill) + ["--help"],
                               capture_output=True, text=True)
    except OSError as e:
        # A missing or non-executable binary must fail the gate, not crash preflight.
        raise Stop(f"{a.events_backfill} cannot be run: {e}") from e
    if "--transport" not in (probe.stdout + probe.stderr):
        raise Stop(
            f"{a.events_backfill} has no --transport flag, so --amm mtls cannot run. "
            "Either merge the events-backfill mTLS transport, or use --amm ssh "
            "(run it on the CH host as `default`) or --amm wait + amm-done.")


# events-backfill's task-0242 line (run.rs UNPROVEN_SAC_LABEL). Absent from a
# pre-0242 binary, which passes (D4: phase 3 is not held for the code).
UNPROVEN_SAC_LABEL = "unproven sac swaps:"


def amm_summary(text, logfile, *, dry_run, unproven_line_seen=False):
    """Read events-backfill's closing summary, or Stop naming what is missing.

    Called after the --dry-run pass as well as the write, so a binary whose
    summary this script cannot read is refused BEFORE it writes — not with a
    Python AttributeError after the month's AMM candles are already in.
    `negative apply order:` replaced `events with no apply order:` in task 0304;
    a binary without it predates that fix and cannot read BE's events anyway.

    `unproven sac swaps:` (task 0242) may be absent: a binary built before 0242
    prints none, and phase 3 runs on one by design, the 0242 seed resolving its
    SACs (D4, PC6). Absent after an earlier month of this state printed it
    (`unproven_line_seen`), the host binary went back to a pre-0242 build: Stop.
    """
    found = {}
    for key, label in (("fallbacks", "negative apply order:"), ("dropped", "swaps dropped (unresolved):")):
        hit = re.search(rf"^{re.escape(label)}\s*(\d+)", text, re.M)
        if not hit:
            raise Stop(f"events-backfill printed no `{label}` line — "
                       f"see {logfile}. A binary without it predates task 0304: rebuild events-backfill "
                       "from develop (on the CH host for --amm ssh)")
        found[key] = int(hit.group(1))
    hit = re.search(rf"^{re.escape(UNPROVEN_SAC_LABEL)}\s*(\d+)", text, re.M)
    found["unproven_sacs"] = int(hit.group(1)) if hit else None
    if found["unproven_sacs"] is None and unproven_line_seen:
        raise Stop(f"events-backfill printed no `{UNPROVEN_SAC_LABEL}` line, but an earlier month of this "
                   "state did: the binary on the host is now one built before task 0242, which mints an "
                   f"unproven SAC as a second identity — see {logfile}. Put the 0242 build back "
                   "(docs/runbooks/0242-sac-identity-heal.md §4e) and `run` again")
    if found["unproven_sacs"]:
        m = Path(logfile).parent.name
        state = (f"This was the --dry-run pass: no AMM candle was written, but {m}'s 1m partition is "
                 "already dropped and holds SDEX-only candles. Fix it today and `run` again, or "
                 f"`rollback {m}`" if dry_run else
                 f"This was the write pass: {m}'s AMM candles are in without those swaps. `rollback {m}`")
        raise Stop(f"events-backfill skipped {found['unproven_sacs']} swaps because their SAC could not be "
                   f"proven (task 0242 D2) — the contracts are in the WARN in {logfile}. {state}: "
                   "docs/runbooks/0242-sac-identity-heal.md §4c")
    return found


def unproven_line_seen(st, m):
    """Whether a month of this state other than `m` recorded `unproven sac swaps:` (task 0242)."""
    return any(v.get("unproven_sacs") is not None for k, v in st.d["months"].items() if k != str(m))


def record_amm_summary(ms, m, summary):
    """Keep the write pass's figures on the month and note the ones that need reading."""
    ms["fallbacks"], ms["dropped"] = summary["fallbacks"], summary["dropped"]
    ms["unproven_sacs"] = summary["unproven_sacs"]
    if ms["unproven_sacs"] is None:
        note(f"{m}: events-backfill printed no `{UNPROVEN_SAC_LABEL}` line — a binary built before task 0242, "
             "so its SACs resolve only through assets.sac_address (the 0242 seed), and an unseeded one "
             "would mint a second identity (heal runbook §5a counts it)")
    if ms["dropped"]:
        note(f"{m}: {ms['dropped']} swaps dropped for unregistered pools — see prices.unresolved_pools")
    if ms["fallbacks"]:
        note(f"{m}: {ms['fallbacks']} events with a negative apply order — their fill order is a fallback, "
             "the range is not repaired (runbook §6)")


def require_amm_path(a):
    """Stop unless the chosen --amm mode can run at all. Runs before any DROP."""
    if a.amm == "mtls":
        require_transport_flag(a)
    elif a.amm == "ssh" and not a.ssh.strip():
        raise Stop("--amm ssh needs --ssh (the ssh target of the CH host, with any options), "
                   "e.g. --ssh '-i ~/.ssh/<key> deploy@<ch-host>'")


def prove_comet_binary(a, ch, m, pw, logfile):
    """Stop unless the events-backfill binary routes venue 'comet' (task 0300, WR-02).

    The pool_registry row is not what makes Comet candles appear: a pre-0300
    binary skips a `venue='comet'` row silently (`Venue::from_source` -> None), so
    a month re-ingested with it loses every Comet swap while the row gate passes.
    Run the same binary the `amm` step will run, as a one-ledger --dry-run over
    COMET_PROBE_LEDGER, and require a non-zero `comet ticks:` line. Runs in the
    gates step, before the snapshot and every DROP. Fails closed.
    """
    global COMET_BINARY_PROVEN
    if COMET_BINARY_PROVEN or a.amm == "stop":
        return
    fix = ("the events-backfill binary predates 0300 — deploy/rebuild it from 0300 (the Comet venue) "
           f"before phase 3 reaches {m}. Nothing was dropped")
    L = str(COMET_PROBE_LEDGER)
    if a.amm == "wait":
        if not a.ack_0300_binary:
            raise Stop(f"{m}: --amm wait cannot probe the host binary for venue '{a.comet_source}': check it "
                       "is built from 0300 or later, then pass --ack-0300-binary")
        COMET_BINARY_PROVEN = True
        return
    if a.amm == "mtls":
        if not ch.dry:
            require_transport_flag(a)
        stem = os.path.expanduser(a.admin_cert)
        env = dict(os.environ, CH_DOMAIN=urllib.parse.urlparse(ch.url).hostname or "",
                   MTLS_CERT_PATH=stem + ".crt", MTLS_KEY_PATH=stem + ".key",
                   MTLS_CA_PATH=os.path.expanduser(a.ca))
        cmd = shlex.split(a.events_backfill) + ["--transport", a.transport, "--start", L, "--end", L,
                                                "--dry-run"]
        stdin_text = None
    else:  # ssh — the host's ~/events-backfill, exactly as the amm step runs it
        env = os.environ
        remote = ("read -r CLICKHOUSE_PASSWORD; export CLICKHOUSE_PASSWORD; exec ~/events-backfill "
                  f"--start {L} --end {L} --clickhouse-url http://localhost:8123 --dry-run")
        cmd = ["ssh"] + shlex.split(a.ssh) + [remote]
        stdin_text = pw
    if ch.dry:
        log(f"[DRY] Comet binary probe: {' '.join(cmd)}")
        return
    code, text = stream(cmd, env, logfile, stdin_text=stdin_text)
    if code != 0 or "=== events-backfill complete ===" not in text:
        raise Stop(f"{m}: the Comet binary probe (events-backfill --dry-run, ledger {L}) exit {code} — "
                   f"see {logfile}. If the binary is the cause: {fix}")
    hit = re.search(rf"^\s*{re.escape(a.comet_source)} ticks:\s*([1-9]\d*)", text, re.M)
    if not hit:
        raise Stop(f"{m}: events-backfill does not route venue '{a.comet_source}': a --dry-run over ledger "
                   f"{L} (Comet's first swap) printed no `{a.comet_source} ticks:` line — {fix}")
    COMET_BINARY_PROVEN = True
    log(f"{m}: events-backfill routes '{a.comet_source}' ({hit.group(1)} tick at ledger {L})")


# ---------------------------------------------------------------- the 12 steps

def run_month(a, ch, st, m, pw):
    ms = st.month(m)
    ms.update(st.d["plan"][str(m)])
    S, E = ms["start"], ms["end"]
    soroban = E >= SOROBAN_ACTIVATION_LEDGER
    post = post0139_mode(ch)
    mdir = st.dir / str(m)
    mdir.mkdir(exist_ok=True)
    t0 = time.time()

    def step(name):
        """True when `name` still has to run; advances the pointer afterwards via done()."""
        DASH.update(month=m, step="", since=time.time(), tail="")
        RERENDER()
        return STEPS[ms["step"]] == name

    def done():
        log(f"{m} {STEPS[ms['step']]} done")
        ms["step"] += 1
        st.save()

    if step("gates"):
        free = int(ch.one("reader", "SELECT min(free_space) FROM system.disks"))
        if free < a.min_free_gb * 2 ** 30:
            raise Stop(f"{free / 2 ** 30:.0f} GiB free < {a.min_free_gb}: release reconciled months "
                       "with `release` before going on")
        if m >= SUSHI_FROM_MONTH:
            n = int(ch.one("reader", f"SELECT count() FROM prices.pool_registry FINAL WHERE venue = '{a.sushi_source}'"))
            if n < SUSHI_POOLS:
                raise Stop(f"{n} {a.sushi_source} pools < {SUSHI_POOLS}: 0290's --discover-pools write has not run")
        if m >= COMET_FROM_MONTH:
            n = int(ch.one("reader", f"SELECT count() FROM prices.pool_registry FINAL WHERE venue = '{a.comet_source}'"))
            if n < COMET_POOLS:
                raise Stop(f"{n} {a.comet_source} pools < {COMET_POOLS}: 0300's --discover-pools write has not run")
        if soroban and a.amm == "stop":
            raise Stop(f"{m} is Soroban-era and --amm stop is set: its AMM candles need events-backfill "
                       "on the CH host. Re-run with --amm ssh or --amm wait")
        if soroban and not ch.dry:
            # Before the snapshot and the DROP: a mode that cannot run would
            # otherwise be found only at the amm step, with the 1m month gone.
            require_amm_path(a)
        if m >= COMET_FROM_MONTH and soroban:
            # The row gate above is not enough: the BINARY routes Comet (WR-02).
            prove_comet_binary(a, ch, m, pw, mdir / "comet-probe.log")
        done()

    if step("snapshot"):
        ms["snap"] = {t: snapshot(ch, f"price_ohlcv_{t}", m) for t in ["1m"] + FINE_TIERS}
        log(f"{m} snapshot {ms['snap']}")
        done()

    if step("before"):
        b = sums(ch, "1m", m, post)
        ms["ref"] = "1m"
        if not b:  # cleanup dropped whole 1m months on 2026-07-18; the coarse copy is the survivor
            b, ms["ref"] = sums(ch, "1h", m, post), "1h"
        ms["before"], ms["ids"] = b, "u64" if post else "u32"
        if post:
            ms["outside_before"] = outside(ch, ms["ref"], m)
        done()

    if step("markers"):
        ch.write("writer", f"ALTER TABLE prices.backfill_sdex_ledgers DELETE WHERE sequence BETWEEN {S} AND {E}")
        deadline = time.time() + 3600
        while not ch.dry:
            pending = int(ch.one("reader", f"""SELECT count() FROM system.mutations WHERE database = '{ch.db}'
                AND table = 'backfill_sdex_ledgers' AND NOT is_done"""))
            left = int(ch.one("reader", f"SELECT count() FROM prices.backfill_sdex_ledgers WHERE sequence BETWEEN {S} AND {E}"))
            DASH["step"] = f"mutations pending {pending}, markers left {left}"
            RERENDER()
            if pending == 0 and left == 0:
                break
            if time.time() > deadline:
                raise Stop("the marker DELETE did not finish in an hour — with markers left the run is a silent no-op")
            time.sleep(5)
        done()

    if step("drop_1m"):
        ch.write("admin", f"ALTER TABLE prices.price_ohlcv_1m DROP PARTITION {m}")
        done()

    if step("sdex"):
        cmd = shlex.split(a.sdex_backfill) + ["--mode", "sdex-only", "--start", str(S), "--end", str(E),
                                              "--tip", str(Ledgers(st.dir / "ledger-cache").tip()),
                                              "--transport", a.transport, "--verbose"]
        env = dict(os.environ, CH_DOMAIN=urllib.parse.urlparse(ch.url).hostname or "", CH_DATABASE=ch.db,
                   MTLS_CERT_PATH=os.path.expanduser(a.writer_cert) + ".crt",
                   MTLS_KEY_PATH=os.path.expanduser(a.writer_cert) + ".key",
                   MTLS_CA_PATH=os.path.expanduser(a.ca))
        if ch.dry:
            log("[DRY] " + " ".join(cmd))
            ms["aligned"] = "dry"
        else:
            code, text = stream(cmd, env, mdir / "sdex-backfill.log")
            # ⚠️ Order matters. sdex-backfill's "all partitions fully indexed" path
            # returns Ok(()) at run.rs:119 and NEVER reaches the completion banner it
            # prints at run.rs:520. Checking for the banner first therefore Stops every
            # resume that lands on a finished month — the exact case the block below is
            # written to handle. Test the idle path first, and keep the banner as the
            # requirement only for a run that actually had work to do.
            idle = "nothing to do" in text
            if code != 0 or ("=== sdex-backfill complete ===" not in text and not idle):
                raise Stop(f"sdex-backfill exit {code} — see {mdir}/sdex-backfill.log (the run is resumable)")
            if idle:
                # Harmless on a resume (the markers are this run's own); fatal when the
                # markers were never cleared, which leaves the dropped month empty.
                had = ms["before"].get("sdex", {}).get("trades", 0)
                now = int(ch.one("reader", f"SELECT count() FROM prices.price_ohlcv_1m WHERE toYYYYMM(timestamp) = {m} AND source = 'sdex'"))
                if had and not now:
                    raise Stop("sdex-backfill found every ledger already marked and the month is EMPTY: "
                               "the markers were not cleared, nothing was re-ingested")
                note(f"{m}: sdex-backfill had nothing left to do — resumed after a finished pass")
            if "S3-incomplete" in text:
                raise Stop("sdex-backfill skipped an S3-incomplete partition: the month is short")
            if "NOT minute-aligned" in text and str(m) not in (a.accept or []):
                raise Stop("run boundary is NOT minute-aligned: rebuild that minute from one pass first")
            ms["aligned"] = f"{text.count('run boundary is minute-aligned')}/2"
        done()

    if step("amm"):
        ms["fallbacks"] = "-"
        if soroban and not ch.dry:
            base = f"~/events-backfill --start {S} --end {E} --clickhouse-url http://localhost:8123 --verbose"
            if a.amm == "mtls":
                # prices_admin reads default.* AND writes prices.*, so the host is not
                # needed — only a binary built with `--features aws-mtls` that also
                # accepts `--transport`.
                #
                # ⚠️ As of 2026-09-22 the events-backfill on `develop` has NO
                # `--transport` flag: it lives on the unpushed branch
                # fix/0286_phase-3-reingest-tooling. sdex-backfill has one, which is
                # what makes the omission easy to miss. Refuse here rather than let a
                # month die on an unknown-argument error after its partition was
                # dropped. The alternatives both work today: `--amm ssh` runs the tool
                # on the CH host as `default` (proven 2026-09-22 by the 0290 pool
                # seed), and `--amm wait` + `amm-done` records a run made by hand.
                require_transport_flag(a)
                stem = os.path.expanduser(a.admin_cert)
                env = dict(os.environ, CH_DOMAIN=urllib.parse.urlparse(ch.url).hostname or "",
                           MTLS_CERT_PATH=stem + ".crt", MTLS_KEY_PATH=stem + ".key",
                           MTLS_CA_PATH=os.path.expanduser(a.ca))
                for flag in (["--dry-run"], []):
                    cmd = shlex.split(a.events_backfill) + ["--transport", a.transport, "--start", str(S),
                                                            "--end", str(E), "--verbose"] + flag
                    code, text = stream(cmd, env, mdir / "events-backfill.log")
                    if code != 0 or "=== events-backfill complete ===" not in text:
                        raise Stop(f"events-backfill {' '.join(flag)} exit {code} — see {mdir}/events-backfill.log")
                    summary = amm_summary(text, mdir / "events-backfill.log", dry_run=bool(flag),
                                          unproven_line_seen=unproven_line_seen(st, m))
                record_amm_summary(ms, m, summary)
            elif a.amm == "ssh":
                for flag in (" --dry-run", ""):
                    remote = f"read -r CLICKHOUSE_PASSWORD; export CLICKHOUSE_PASSWORD; exec {base}{flag}"
                    code, text = stream(["ssh"] + shlex.split(a.ssh) + [remote], os.environ,
                                        mdir / "events-backfill.log", stdin_text=pw)
                    if code != 0 or "=== events-backfill complete ===" not in text:
                        raise Stop(f"events-backfill{flag} exit {code} — see {mdir}/events-backfill.log")
                    summary = amm_summary(text, mdir / "events-backfill.log", dry_run=bool(flag),
                                          unproven_line_seen=unproven_line_seen(st, m))
                record_amm_summary(ms, m, summary)
            else:
                marker = mdir / "amm.done"
                note(f"{m}: waiting for the host run —  read -rs CH_PW; CLICKHOUSE_PASSWORD=\"$CH_PW\" {base}"
                     f"   then: reingest_0286.py amm-done {m} --fallbacks <negative apply order>")
                while not marker.exists():
                    DASH["step"] = "waiting for amm-done"
                    RERENDER()
                    time.sleep(15)
                ms["fallbacks"] = int(marker.read_text().strip() or 0)
        done()

    if step("reconcile"):
        if ms.get("ids", "u32") != ("u64" if post else "u32") and str(m) not in (a.accept or []):
            raise Stop(f"{m}: its before numbers were read on {ms.get('ids', 'u32')} ids and the 0139 window "
                       f"changed the width since. Compare by hand, then `run --accept {m}` with the reason on "
                       f"the task, or re-ingest the month in the second pass")
        after = ms["before"] if ch.dry else sums(ch, "1m", m, post)
        ms["after"] = after
        if post and not ch.dry:
            ob, ms["outside_after"] = ms.get("outside_before", {}), outside(ch, "1m", m)
            for k in sorted(set(ob) | set(ms["outside_after"])):
                note(f"{m} {k}: {ob.get(k, 0)} -> {ms['outside_after'].get(k, 0)} trades on ids the 0139 rekey "
                     "did not copy, back under their own identities — information, not reconciled")
        rank, amm_bits = 0, []  # 0 OK, 1 FINDING, 2 DEFECT
        damaged = m in LIVE_LOSS_MONTHS
        for src in sorted(set(ms["before"]) | set(after)):
            b = ms["before"].get(src, {"trades": 0, "vb": "0", "vq": "0"})
            x = after.get(src, {"trades": 0, "vb": "0", "vq": "0"})
            less = x["trades"] < b["trades"] or Decimal(x["vb"]) < Decimal(b["vb"]) or Decimal(x["vq"]) < Decimal(b["vq"])
            same = (x["trades"], Decimal(x["vb"]), Decimal(x["vq"])) == (b["trades"], Decimal(b["vb"]), Decimal(b["vq"]))
            gain = (x["trades"] - b["trades"]) / b["trades"] * 100 if b["trades"] else 0.0
            if less:
                rank = 2
                note(f"{m} {src}: LESS than the snapshot ({b['trades']} -> {x['trades']} trades)")
            elif damaged and same and b["trades"]:
                rank = max(rank, 1)
                note(f"{m} {src}: EQUAL inside 2026-07-16..09-17, where the snapshot is the damaged side (0282)")
            elif not b["trades"] and x["trades"]:
                rank = max(rank, 1)
                note(f"{m} {src}: absent from the snapshot, {x['trades']} trades now — nothing to reconcile against")
            elif src == "sdex" and not damaged and not same:
                rank = max(rank, 2 if gain > a.sdex_max_gain_pct else 1)
                note(f"{m} sdex: +{gain:.4f} % — expected EQUAL to the stroop bar the old backfill's "
                     "64k partition-boundary minutes; account for it")
            if src != "sdex":
                amm_bits.append(f"{src[:4]}{gain:+.1f}%")
        if m >= SUSHI_FROM_MONTH and not ch.dry and after.get(a.sushi_source, {}).get("trades", 0) == 0:
            rank = 2
            note(f"{m}: no {a.sushi_source} volume — the ORDER slipped (0290's registry write), the data is not bad")
        no_comet = not ch.dry and after.get(a.comet_source, {}).get("trades", 0) == 0
        if COMET_FROM_MONTH <= m <= COMET_MEASURED_THROUGH and no_comet:
            # Measured: this month holds >= 135 real Comet swaps (see COMET_MEASURED_THROUGH).
            rank = 2
            note(f"{m}: no {a.comet_source} volume — the ORDER slipped (the events-backfill binary predates "
                 "0300, or 0300's registry write is missing), the data is not bad")
        elif m > COMET_MEASURED_THROUGH and no_comet:
            rank = max(rank, 1)
            note(f"{m}: no {a.comet_source} volume in an unmeasured month — the pool is frozen and winding "
                 "down since the 2026-08-25 exploit and may simply be idle; the binary and the registry row "
                 "were proven in the gates step")
        if m >= 202609 and amm_bits:
            note(f"{m}: an AMM gain here includes pools seeded mid-month (0291 on 09-18 08:00 UTC, then 0290) — "
                 "the re-ingest resolves pool_registry as of now, so it is not loss repaired")
        verdict = ["OK", "FINDING", "DEFECT"][rank]
        bs, xs = ms["before"].get("sdex", {}).get("trades", 0), after.get("sdex", {}).get("trades", 0)
        ms["result"] = {"ref": ms["ref"], "sdex": f"{bs} -> {xs}", "verdict": verdict,
                        "amm": " ".join(amm_bits) or "-", "aligned": ms.get("aligned", "-"),
                        "fallbacks": ms.get("fallbacks", "-")}
        st.save()
        if verdict == "DEFECT" and str(m) not in (a.accept or []):
            raise Stop(f"{m}: reconciliation failed — the coarse partitions are UNTOUCHED. Investigate, then "
                       f"`rollback {m}` or `run --accept {m}` with the reason written on the task")
        done()

    if step("drop_coarse"):
        for t in FINE_TIERS:
            ch.write("admin", f"ALTER TABLE prices.price_ohlcv_{t} DROP PARTITION {m}")
        done()

    if step("preroll"):
        gap = Path(a.repo) / "packages/prices-clickhouse/schema/preroll-live-gap.sql"
        params = {"start_ts": f"{month_start(m):%Y-%m-%d} 00:00:00",
                  "end_ts": f"{month_start(next_month(m)):%Y-%m-%d} 00:00:00"}
        for t, sql in zip(FINE_TIERS, statements(gap, FINE_TIERS)):
            DASH["step"] = f"pre-roll {t}"
            RERENDER()
            ch.write("writer", sql, params=params, timeout=86400,
                     settings={"max_bytes_before_external_group_by": 4_000_000_000})
        done()

    if step("verify"):
        for t in FINE_TIERS:
            bad = 0 if ch.dry else int(ch.one("reader", f"""SELECT count() FROM prices.price_ohlcv_{t} FINAL
                WHERE toYYYYMM(timestamp) = {m}
                  AND NOT (low <= open AND low <= close AND open <= high AND close <= high)""", timeout=3600))
            if bad:
                raise Stop(f"{m} {t}: {bad} OHLC-order violations — there is no clamp, so the pre-roll read a mixed subset")
        done()

    if step("done"):
        ms["seconds"] = ms.get("seconds", 0) + time.time() - t0
        r = ms["result"]
        with open(os.devnull if ch.dry else st.dir / "results.tsv", "a") as f:
            f.write("\t".join(map(str, [m, S, E, r["ref"], r["sdex"], r["verdict"], r["amm"], r["aligned"],
                                        r["fallbacks"], round(ms["seconds"])])) + "\n")
        done()


def cmd_run(a, ch, st):
    global RERENDER
    months = planned_months(a, st, ch)
    st.lock()
    print("preflight:")
    cmd_preflight(a, ch, st, months)
    todo = [m for m in months if st.month(m).get("step", 0) < len(STEPS)]
    print(f"\n{len(todo)} months to go, {todo[0] if todo else '-'} first. Each month: snapshot -> clear markers -> "
          f"DROP PARTITION -> re-ingest -> reconcile -> DROP coarse -> pre-roll.\nTarget: {ch.url} / {ch.db}"
          f"{'   [DRY RUN — nothing is written]' if ch.dry else ''}")
    if not a.yes and not ch.dry:
        if input(f"Type the database name to start dropping partitions on it: ") != ch.db:
            raise Stop("not confirmed")
    soroban_due = any(st.d["plan"][str(m)]["end"] >= SOROBAN_ACTIVATION_LEDGER for m in todo)
    pw = getpass.getpass("CH `default` password for events-backfill (stdin only, never argv): ") \
        if a.amm == "ssh" and soroban_due and not ch.dry else None
    RERENDER = lambda: render(st, months)
    for m in todo:
        run_month(a, ch, st, m, pw)
    DASH["month"] = None
    print(render(st, months, clear=False))
    print("\nEvery month is done. Next: `finish` (1w + 1M, then the acceptance reads).")


def cmd_status(a, ch, st):
    months = planned_months(a, st, ch)
    cur = [m for m in months if 0 < st.month(m).get("step", 0) < len(STEPS)]
    DASH["month"] = cur[0] if cur else None
    print(render(st, months, clear=False))


def cmd_amm_done(a, ch, st):
    (st.dir / str(a.month)).mkdir(exist_ok=True)
    (st.dir / str(a.month) / "amm.done").write_text(str(a.fallbacks))


def cmd_finish(a, ch, st):
    months = planned_months(a, st, ch)
    left = [m for m in months if st.month(m).get("step", 0) < len(STEPS)]
    if left:
        raise Stop(f"{len(left)} months are not done ({left[0]}..): a week straddles months, so 1w waits for all of them")
    for t in ("1w", "1M"):
        for (p,) in ch.rows("reader", f"""SELECT DISTINCT partition FROM system.parts WHERE database = '{ch.db}'
                AND table = 'price_ohlcv_{t}' AND active ORDER BY partition"""):
            print(f"snapshot {t} {p}: {snapshot(ch, f'price_ohlcv_{t}', p)}", flush=True)
    pre = Path(a.repo) / "packages/prices-clickhouse/schema/preroll.sql"
    for t, sql in zip(("1w", "1M"), statements(pre, ["1w", "1M"])):
        ch.write("admin", f"TRUNCATE TABLE prices.price_ohlcv_{t}")
        # max_partitions_per_insert_block: the full-range pre-roll writes one block
        # spanning every month of history — ~131 partitions against a default cap of
        # 100, which fails `Code: 252 TOO_MANY_PARTS` AFTER the TRUNCATE above. Hit on
        # production 2026-09-22 during phase 1's own 1M rebuild (task 0302).
        ch.write("writer", sql, timeout=86400,
                 settings={"max_bytes_before_external_group_by": 4_000_000_000,
                           "max_partitions_per_insert_block": 1000})
        print(f"{t} rebuilt", flush=True)
    for t in FINE_TIERS + ["1w", "1M"]:
        bad = ch.one("reader", f"""SELECT count() FROM prices.price_ohlcv_{t} FINAL
            WHERE NOT (low <= open AND low <= close AND open <= high AND close <= high)""", timeout=7200)
        print(f"acceptance 2 — OHLC order on {t}: {bad} violations", flush=True)
    print("""
Not automatic, by design (runbook §7b-7e, §8):
  7b  the enrichment worker now re-prices the whole history on its own (hours). Capture
      POST_RUN_0228_VERSION_BEFORE_1W / _1M BEFORE it starts.
  7c  cargo test -p enrichment-worker --test post_run_0228_it -- --ignored
  7e  the USDT-peg read (USDT by identity, runbook §7e): peg_written = 0, pivot_written > 0, on 1m and 1h
  8.3 XLM/USDC 1d closes on the seven dust days within 5 % of Bitstamp
  8.4 candles priced from a quantised XLM/USDC close: zero on 15m/1h/4h/1d/1w""")


def cmd_rollback(a, ch, st):
    m = a.month
    snap = st.month(m).get("snap")
    if not snap:
        raise Stop(f"{m} has no snapshot on record in {st.f} — nothing to roll back to")
    for t, what in snap.items():  # every check before the first write
        bak, table = f"{BAK}price_ohlcv_{t}", f"price_ohlcv_{t}"
        if what == "empty":
            continue
        have, want = id_types(ch, bak), id_types(ch, table)
        if have != want:
            raise Stop(f"{m} {t}: prices.{bak} holds {have.get('asset_id', 'no')} ids, prices.{table} "
                       f"{want.get('asset_id')}: REPLACE PARTITION "
                       f"cannot cross id spaces. {PRE0139.format(bak=bak, table=table)}. Nothing was written")
        if not part_rows(ch, bak, m):
            raise Stop(f"{m} {t}: prices.{bak} holds no {m} rows (released, or snapshotted before 0139), so "
                       "REPLACE PARTITION would empty the month. "
                       f"{PRE0139.format(bak=bak, table=table)}. Nothing was written")
    for t, what in snap.items():
        if what == "empty":  # the partition did not exist before the run
            ch.write("admin", f"ALTER TABLE prices.price_ohlcv_{t} DROP PARTITION {m}")
        else:
            ch.write("admin", f"ALTER TABLE prices.price_ohlcv_{t} REPLACE PARTITION {m} "
                              f"FROM prices.{BAK}price_ohlcv_{t}")
        print(f"{t} {m}: restored ({what})", flush=True)
    st.d["months"].pop(str(m), None)
    st.save()
    print("The completion markers are NOT restored — the restored rows are already the indexed result.")


def cmd_release(a, ch, st):
    if st.month(a.month).get("step", 0) < len(STEPS):
        raise Stop(f"{a.month} is not reconciled and done — its snapshot is its only rollback")
    for t in ["1m"] + FINE_TIERS:
        if not part_rows(ch, f"{BAK}price_ohlcv_{t}", a.month):
            print(f"{t} {a.month}: no snapshot rows (one taken before 0139 is in {BAK}price_ohlcv_{t}_pre0139, "
                  "dropped whole after the second pass)", flush=True)
            continue
        ch.write("admin", f"ALTER TABLE prices.{BAK}price_ohlcv_{t} DROP PARTITION {a.month}")


def build_parser():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("command", choices=["plan", "preflight", "run", "status", "amm-done", "finish", "rollback", "release"])
    p.add_argument("month", nargs="?", type=int)
    p.add_argument("--from-month", type=int)
    p.add_argument("--to-month", type=int)
    p.add_argument("--dry-run", action="store_true", help="reads run, every write and child process is only printed")
    p.add_argument("--yes", action="store_true")
    p.add_argument("--accept", action="append", help="YYYYMM whose failed reconciliation is understood and recorded")
    p.add_argument("--fallbacks", type=int, default=0)
    p.add_argument("--ack-phase1-measured", action="store_true")
    p.add_argument("--ack-0285", action="store_true")
    p.add_argument("--ack-0300-binary", action="store_true",
                   help="--amm wait only: the host events-backfill is built from 0300 or later (routes Comet)")
    p.add_argument("--ack-0139-binaries", action="store_true",
                   help="after 0139: sdex-backfill and events-backfill (local and on the CH host) are rebuilt "
                        "from the 0139 merge or later")
    p.add_argument("--second-pass", action="store_true",
                   help="after 0139: only the months in prices.rekey_0139_reingest_months, up to --to-month")
    p.add_argument("--min-excluded-rows", type=int, default=0,
                   help="--second-pass: skip, and log, listed months with at most N excluded 1m rows")
    # ssh, not mtls: events-backfill on develop has no --transport (runbook §6).
    p.add_argument("--amm", choices=["mtls", "stop", "ssh", "wait"], default="ssh")
    p.add_argument("--events-backfill", default="./target/release/events-backfill")
    p.add_argument("--ssh", default="", help="ssh target (and options) of the CH host, for --amm ssh")
    p.add_argument("--state-dir", default="~/reingest-0286")
    p.add_argument("--repo", default=subprocess.run(["git", "rev-parse", "--show-toplevel"],
                                                    capture_output=True, text=True).stdout.strip() or ".")
    p.add_argument("--ch-url", default="https://ch.sorobanscan.rumblefish.dev")
    p.add_argument("--database", default="prices")
    # Defaults are the bundles ON the campaign machine (fishuser-hero), per the
    # 2026-09-21 decision: prices_admin, NOT dev_shared. The admin cert is the
    # READER too — it is uncapped, while dev_read's 30 s / 4 GB profile cannot
    # carry the month-wide FINAL sums these checks run.
    p.add_argument("--admin-cert", default="~/prices-mtls/prices-admin-production")
    p.add_argument("--writer-cert", default="~/prices-mtls/prices_writer")
    p.add_argument("--reader-cert", default="~/prices-mtls/prices-admin-production")
    p.add_argument("--ca", default="~/prices-mtls/ca.crt")
    p.add_argument("--sdex-backfill", default="./target/release/sdex-backfill")
    p.add_argument("--transport", default="hetzner")
    p.add_argument("--cleanup-rule", default="prices-production-cleanup")
    p.add_argument("--skip-aws-check", action="store_true")
    p.add_argument("--sushi-source", default="sushiswap")
    p.add_argument("--comet-source", default="comet")
    p.add_argument("--min-free-gb", type=int, default=300)
    p.add_argument("--sdex-max-gain-pct", type=float, default=0.5)
    return p


def main():
    p = build_parser()
    a = p.parse_args()
    if a.command in ("amm-done", "rollback", "release") and not a.month:
        p.error(f"{a.command} needs a month")
    if a.min_excluded_rows and not a.second_pass:
        p.error("--min-excluded-rows applies to --second-pass only")
    global LOG
    st = State(a.state_dir, a.dry_run)
    LOG = st.dir / "run.log"
    try:
        ch = CH(a)
        {"plan": cmd_plan, "preflight": cmd_preflight, "run": cmd_run, "status": cmd_status,
         "amm-done": cmd_amm_done, "finish": cmd_finish, "rollback": cmd_rollback,
         "release": cmd_release}[a.command](a, ch, st)
    except Stop as e:
        log(f"STOP {e}")
        print(f"\nSTOP — {e}", file=sys.stderr)
        sys.exit(2)
    except ClickHouseDown as e:
        log(f"DOWN {e}")
        print(f"\nDOWN — {e}\nstate kept; `run` resumes at the same step", file=sys.stderr)
        sys.exit(1)
    except KeyboardInterrupt:
        print("\ninterrupted — state kept, `run` resumes at the same step", file=sys.stderr)
        sys.exit(130)


if __name__ == "__main__":
    main()
