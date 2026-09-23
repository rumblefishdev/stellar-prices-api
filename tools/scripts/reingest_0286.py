#!/usr/bin/env python3
"""Task 0286 phase 3 — re-ingest the candle history, oldest month first.

Automates the month loop of docs/runbooks/0286-reingest-history.md and STOPS at
every gate that runbook names. Standard library only.

  plan       month -> ledger range, read from the Stellar history archive
  preflight  the runbook's preconditions, plus 0290's pool-registry gate
  run        the month loop (resumable; one month = twelve steps)
  status     the dashboard, once (use under `watch -n5` in a second pane)
  amm-done   record an events-backfill run someone did on the host (--amm wait)
             — the default, --amm mtls, runs events-backfill here and needs nobody
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
BAK = "reingest_0286_bak_"


class Stop(Exception):
    """A gate said no. State is kept; fix the cause and `run` again."""


# ---------------------------------------------------------------- ClickHouse

class CH:
    def __init__(self, a):
        self.url, self.db, self.dry = a.ch_url.rstrip("/"), a.database, a.dry_run
        self.ctx = {}
        if self.url.startswith("https"):
            for role, stem in (("admin", a.admin_cert), ("writer", a.writer_cert),
                               ("reader", a.reader_cert)):
                c = ssl.create_default_context(cafile=os.path.expanduser(a.ca))
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
        req = urllib.request.Request(url, data=self._sql(sql).encode(), method="POST")
        try:
            with urllib.request.urlopen(req, timeout=timeout, context=self.ctx.get(role)) as r:
                return r.read().decode()
        except urllib.error.HTTPError as e:
            raise Stop(f"ClickHouse refused ({role}): {e.read().decode()[:600]}\n--- {sql[:300]}")

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


def log(msg):
    line = f"{dt.datetime.now(UTC):%Y-%m-%d %H:%M:%S}Z {msg}"
    if LOG:
        with open(LOG, "a") as f:
            f.write(line + "\n")
    if not sys.stdout.isatty():
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


def planned_months(a, st):
    ms = sorted(int(m) for m in st.d["plan"])
    if not ms:
        raise Stop("no plan — run `plan` first")
    return [m for m in ms if (not a.from_month or m >= a.from_month)
            and (not a.to_month or m <= a.to_month)]


# ----------------------------------------------------------------- preflight

def cmd_preflight(a, ch, st, months=None):
    months = months or planned_months(a, st)
    ok = []

    def gate(name, passed, detail):
        ok.append(passed)
        print(f"  {'ok  ' if passed else 'STOP'} {name}: {detail}", flush=True)

    v = ch.rows("reader", "SELECT version(), timezone(), serverTimezone()")[0]
    gate("server", v[1] == "UTC" and v[2] == "UTC", " / ".join(v))
    n = int(ch.one("reader", f"""SELECT count() FROM system.columns WHERE database = '{ch.db}'
        AND table LIKE 'price_ohlcv_%' AND name IN ('pf_trade_count','pf_volume','pf_price_volume')"""))
    gate("phase 1 schema", n == 21, f"{n}/21 pf columns")
    if not ch.dry:
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
             "Earlier, the ~51.6k Comet swaps are silently absent again")
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
        print("  note AMM path: " +
             {"mtls": f"--amm mtls: {a.events_backfill} --transport {a.transport}, as the admin certificate "
                      "(cargo build --release -p events-backfill --features aws-mtls)",
              "stop": "--amm stop: the loop halts before the first Soroban-era month",
              "ssh": f"--amm ssh: events-backfill on {a.ssh} as `default`",
              "wait": "--amm wait: pauses for someone with host access, then `amm-done`"}[a.amm], flush=True)
    if not all(ok):
        raise Stop("preflight failed")


# ------------------------------------------------------------- month helpers

SUMS = """SELECT source, count(), sum(trade_count), sum(volume_base), sum(volume_quote)
FROM prices.price_ohlcv_{tier} FINAL WHERE toYYYYMM(timestamp) = {m}
GROUP BY source ORDER BY source"""


def sums(ch, tier, m):
    return {r[0]: {"candles": int(r[1]), "trades": int(r[2]), "vb": r[3], "vq": r[4]}
            for r in ch.rows("reader", SUMS.format(tier=tier, m=m), timeout=3600)}


def part_rows(ch, table, m):
    return int(ch.one("reader", f"""SELECT sum(rows) FROM system.parts WHERE database = '{ch.db}'
        AND table = '{table}' AND active AND partition = '{m}'"""))


def snapshot(ch, table, m):
    """Hardlink the partition into a backup table. SQL-only, so it restores over mTLS.

    The runbooks' `ATTACH PARTITION … FROM '/var/lib/clickhouse/shadow/…'` is a syntax
    error on 26.3.10.60 (FROM takes a table), and a FREEZE can only be restored by
    copying files on the host — which this operator has no SSH for.
    """
    bak = BAK + table
    ch.write("admin", f"CREATE TABLE IF NOT EXISTS prices.{bak} AS prices.{table}")
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


# ---------------------------------------------------------------- the 12 steps

def run_month(a, ch, st, m, pw):
    ms = st.month(m)
    ms.update(st.d["plan"][str(m)])
    S, E = ms["start"], ms["end"]
    soroban = E >= SOROBAN_ACTIVATION_LEDGER
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
        done()

    if step("snapshot"):
        ms["snap"] = {t: snapshot(ch, f"price_ohlcv_{t}", m) for t in ["1m"] + FINE_TIERS}
        log(f"{m} snapshot {ms['snap']}")
        done()

    if step("before"):
        b = sums(ch, "1m", m)
        ms["ref"] = "1m"
        if not b:  # cleanup dropped whole 1m months on 2026-07-18; the coarse copy is the survivor
            b, ms["ref"] = sums(ch, "1h", m), "1h"
        ms["before"] = b
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
                probe = subprocess.run(shlex.split(a.events_backfill) + ["--help"],
                                       capture_output=True, text=True)
                if "--transport" not in (probe.stdout + probe.stderr):
                    raise Stop(
                        f"{a.events_backfill} has no --transport flag, so --amm mtls cannot run. "
                        "Either merge the events-backfill mTLS transport, or use --amm ssh "
                        "(run it on the CH host as `default`) or --amm wait + amm-done.")
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
                ms["fallbacks"] = int(re.search(r"events with no apply order:\s*(\d+)", text).group(1))
                ms["dropped"] = int(re.search(r"swaps dropped \(unresolved\):\s*(\d+)", text).group(1))
                if ms["dropped"]:
                    note(f"{m}: {ms['dropped']} swaps dropped for unregistered pools — see prices.unresolved_pools")
            elif a.amm == "ssh":
                for flag in (" --dry-run", ""):
                    remote = f"read -r CLICKHOUSE_PASSWORD; export CLICKHOUSE_PASSWORD; exec {base}{flag}"
                    code, text = stream(["ssh"] + shlex.split(a.ssh) + [remote], os.environ,
                                        mdir / "events-backfill.log", stdin_text=pw)
                    if code != 0 or "=== events-backfill complete ===" not in text:
                        raise Stop(f"events-backfill{flag} exit {code} — see {mdir}/events-backfill.log")
                ms["fallbacks"] = int(re.search(r"events with no apply order:\s*(\d+)", text).group(1))
            else:
                marker = mdir / "amm.done"
                note(f"{m}: waiting for the host run —  read -rs CH_PW; CLICKHOUSE_PASSWORD=\"$CH_PW\" {base}"
                     f"   then: reingest_0286.py amm-done {m} --fallbacks <events with no apply order>")
                while not marker.exists():
                    DASH["step"] = "waiting for amm-done"
                    RERENDER()
                    time.sleep(15)
                ms["fallbacks"] = int(marker.read_text().strip() or 0)
        done()

    if step("reconcile"):
        after = ms["before"] if ch.dry else sums(ch, "1m", m)
        ms["after"] = after
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
        # Every month since 2024-05 holds >= 135 real Comet swaps (measured 2026-09-23).
        if m >= COMET_FROM_MONTH and not ch.dry and after.get(a.comet_source, {}).get("trades", 0) == 0:
            rank = 2
            note(f"{m}: no {a.comet_source} volume — the ORDER slipped (0300's registry write), the data is not bad")
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
    months = planned_months(a, st)
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
    pw = getpass.getpass("CH `default` password for events-backfill (stdin only, never argv): ") \
        if a.amm == "ssh" and not ch.dry else None
    RERENDER = lambda: render(st, months)
    for m in todo:
        run_month(a, ch, st, m, pw)
    DASH["month"] = None
    print(render(st, months, clear=False))
    print("\nEvery month is done. Next: `finish` (1w + 1M, then the acceptance reads).")


def cmd_status(a, ch, st):
    months = planned_months(a, st)
    cur = [m for m in months if 0 < st.month(m).get("step", 0) < len(STEPS)]
    DASH["month"] = cur[0] if cur else None
    print(render(st, months, clear=False))


def cmd_amm_done(a, ch, st):
    (st.dir / str(a.month)).mkdir(exist_ok=True)
    (st.dir / str(a.month) / "amm.done").write_text(str(a.fallbacks))


def cmd_finish(a, ch, st):
    months = planned_months(a, st)
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
  7e  the USDT-peg read (quote_asset_id = 111): peg_written = 0, pivot_written > 0, on 1m and 1h
  8.3 XLM/USDC 1d closes on the seven dust days within 5 % of Bitstamp
  8.4 candles priced from a quantised XLM/USDC close: zero on 15m/1h/4h/1d/1w""")


def cmd_rollback(a, ch, st):
    m = a.month
    snap = st.month(m).get("snap")
    if not snap:
        raise Stop(f"{m} has no snapshot on record in {st.f} — nothing to roll back to")
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
        ch.write("admin", f"ALTER TABLE prices.{BAK}price_ohlcv_{t} DROP PARTITION {a.month}")


def main():
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
    p.add_argument("--amm", choices=["mtls", "stop", "ssh", "wait"], default="mtls")
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
    a = p.parse_args()
    if a.command in ("amm-done", "rollback", "release") and not a.month:
        p.error(f"{a.command} needs a month")
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
    except KeyboardInterrupt:
        print("\ninterrupted — state kept, `run` resumes at the same step", file=sys.stderr)
        sys.exit(130)


if __name__ == "__main__":
    main()
