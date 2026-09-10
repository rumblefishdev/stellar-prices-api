#!/usr/bin/env python3
"""Phase 4: invariants that reject a synthetic stablecoin series, with the
thresholds derived from real data and the tests that prove they separate.

Thresholds (why each number — measured 2026-09-04 with the snippet at the
bottom of this file over Chainlink USDC/USDT/DAI, Bitstamp, Kraken, OKX daily
bars; see notes/S-guardrails.md for the table):

  ZERO_TRADES_SHARE_30D = 0.9
      share of candles in a 30-day window with trade_count == 0. Real venues:
      0.000 on every window. Our USDC: 1.000 on every window. The sweep of
      232 assets found exactly one asset above 0.5 (USDC). This is THE
      discriminator; the price-shaped ones below are corroboration.

  FLAT_AND_NO_TRADES_SHARE_30D = 0.9
      share of candles with O=H=L=C AND trade_count == 0. Flatness alone is
      NOT a signal: Chainlink daily bars built from a single heartbeat round
      are flat on 68 % of days (USDT: 88 %), and 20 thin Stellar assets with
      one trade a day are honestly flat. Joined with trade_count == 0 the
      real-data share is 0.

  EXACT_PEG_SHARE_30D = 0.9
      share of closes exactly == 1.0 in a 30-day window. Real max over any
      window: 0.67 (Kraken, which prints 4 decimals), 0.47 (OKX), 0.40
      (Bitstamp, Chainlink). Ours: 1.00 for 1 864 consecutive days. 0.9 sits
      above the noisiest real window with margin and below ours by 0.1.

  CONSTANT_CLOSE_RUN_DAYS = 30
      longest run of identical consecutive closes. Real max: 18 days
      (Kraken, 4 dp), 10 (OKX), 4 (Chainlink, Bitstamp). Ours: 1 864.

  MIN_RVOL_30D_BPS = 0.1
      30-day realized vol of daily log returns. Real minimum over every
      window: 0.23 bps (Bitstamp), 0.33 (Chainlink), 0.58 (Kraken), 0.85
      (OKX); no real window is ever exactly zero. Ours: 0 on every window
      before 2026-03-11. 0.1 is less than half the quietest real window.

  ZERO_VOLUME_SHARE_30D = 0.9
      share of candles with volume == 0. Real venues: 0. Ours: 1.

Each check returns (ok, value). `check_series` runs them all and returns the
list of violations. `python3 guardrails.py` runs the self-tests: our USDC
series must FAIL, every real series must PASS, and a stress window must not
trigger anything.
"""
from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

import numpy as np
import pandas as pd

DATA = Path(__file__).resolve().parent.parent / "data"
WINDOW = 30
ZERO_TRADES_SHARE_30D = 0.9
FLAT_AND_NO_TRADES_SHARE_30D = 0.9
EXACT_PEG_SHARE_30D = 0.9
CONSTANT_CLOSE_RUN_DAYS = 30
MIN_RVOL_30D_BPS = 0.1
ZERO_VOLUME_SHARE_30D = 0.9


@dataclass
class Violation:
    check: str
    value: float
    threshold: float
    where: str

    def __str__(self) -> str:
        return f"{self.check}: {self.value:.3f} vs {self.threshold} ({self.where})"


def _longest_run(s: pd.Series) -> int:
    groups = (s != s.shift()).cumsum()
    return int(s.groupby(groups).size().max()) if len(s) else 0


def check_series(df: pd.DataFrame, window: int = WINDOW) -> list[Violation]:
    """df: daily candles with columns open, high, low, close and, where the
    surface has them, trade_count and volume. Missing columns skip their check."""
    v: list[Violation] = []
    close = df["close"].astype(float)
    if len(close) < window:
        return v

    if "trade_count" in df:
        s = (df["trade_count"].fillna(0) == 0).astype(float).rolling(window).mean()
        if s.max() >= ZERO_TRADES_SHARE_30D:
            v.append(Violation("zero_trades_share_30d", float(s.max()), ZERO_TRADES_SHARE_30D, str(s.idxmax().date())))
        flat = (df["open"] == df["high"]) & (df["high"] == df["low"]) & (df["low"] == df["close"])
        s = (flat & (df["trade_count"].fillna(0) == 0)).astype(float).rolling(window).mean()
        if s.max() >= FLAT_AND_NO_TRADES_SHARE_30D:
            v.append(Violation("flat_and_no_trades_share_30d", float(s.max()), FLAT_AND_NO_TRADES_SHARE_30D, str(s.idxmax().date())))

    s = (close == 1.0).astype(float).rolling(window).mean()
    if s.max() >= EXACT_PEG_SHARE_30D:
        v.append(Violation("exact_peg_share_30d", float(s.max()), EXACT_PEG_SHARE_30D, str(s.idxmax().date())))

    run = _longest_run(close.round(10))
    if run > CONSTANT_CLOSE_RUN_DAYS:
        v.append(Violation("constant_close_run_days", run, CONSTANT_CLOSE_RUN_DAYS, "longest run"))

    lr = np.log(close / close.shift(1)).replace([np.inf, -np.inf], np.nan)
    rv = lr.rolling(window).std() * 1e4
    if rv.dropna().min() < MIN_RVOL_30D_BPS:
        v.append(Violation("min_rvol_30d_bps", float(rv.dropna().min()), MIN_RVOL_30D_BPS, str(rv.idxmin().date())))

    if "volume" in df:
        s = (pd.to_numeric(df["volume"], errors="coerce").fillna(0) == 0).astype(float).rolling(window).mean()
        if s.max() >= ZERO_VOLUME_SHARE_30D:
            v.append(Violation("zero_volume_share_30d", float(s.max()), ZERO_VOLUME_SHARE_30D, str(s.idxmax().date())))
    return v


# ----------------------------------------------------------------------- tests
def _load(name: str) -> pd.DataFrame:
    df = pd.read_csv(DATA / name, parse_dates=["ts"]).set_index("ts").sort_index()
    if "volume_base" in df:
        df = df.rename(columns={"volume_base": "volume"})
    # A venue that does not publish a field leaves it empty; an absent
    # column skips its check, an all-zero one would fire it.
    return df.dropna(axis=1, how="all")


def test_our_usdc_series_fails() -> None:
    v = check_series(_load("our_USDC_1d.csv"))
    names = {x.check for x in v}
    assert "zero_trades_share_30d" in names, v
    assert "exact_peg_share_30d" in names, v
    assert "constant_close_run_days" in names, v
    assert "min_rvol_30d_bps" in names, v


def test_our_native_control_passes_price_checks() -> None:
    # XLM is not a stablecoin; only the price-shaped checks apply and none may fire.
    v = check_series(_load("our_native_1d.csv"))
    assert not v, v


def test_real_usd_sources_pass() -> None:
    for name in ["chainlink_usdc_1d.csv", "bitstamp_usd_1d.csv", "kraken_usd_1d_last720.csv", "okx_usdt_1d.csv",
                 "chainlink_usdt_1d.csv", "chainlink_dai_1d.csv", "composed_usdc_usd_1d.csv"]:
        df = _load(name)
        if "trades" in df:
            df = df.rename(columns={"trades": "trade_count"})
        if "n_obs" in df:
            df = df.rename(columns={"n_obs": "trade_count"})
        v = [x for x in check_series(df) if x.check not in ("zero_volume_share_30d",)]  # oracle bars carry no volume
        assert not v, f"{name}: {v}"


def test_stress_window_does_not_trigger() -> None:
    df = _load("chainlink_usdc_1d.csv").rename(columns={"trades": "trade_count"})
    win = df[(df.index >= "2023-02-01") & (df.index < "2023-04-30")]
    assert not check_series(win), check_series(win)


def test_a_quiet_real_quarter_does_not_trigger() -> None:
    # The quietest 30-day window on Bitstamp (rvol 0.23 bps) must pass.
    df = _load("bitstamp_usd_1d.csv")
    lr = np.log(df["close"] / df["close"].shift(1))
    end = (lr.rolling(WINDOW).std()).idxmin()
    win = df[(df.index > end - pd.Timedelta(days=WINDOW + 5)) & (df.index <= end)]
    assert not check_series(win), check_series(win)


def test_synthetic_injection_is_caught() -> None:
    # A real series with a 45-day 1.0000 splice inserted must fail on the run check.
    df = _load("bitstamp_usd_1d.csv").copy()
    i = len(df) // 2
    df.iloc[i:i + 45, df.columns.get_loc("close")] = 1.0
    names = {x.check for x in check_series(df)}
    assert "constant_close_run_days" in names, names


if __name__ == "__main__":
    import sys
    fails = 0
    for name, fn in list(globals().items()):
        if name.startswith("test_") and callable(fn):
            try:
                fn()
                print(f"PASS {name}")
            except AssertionError as e:
                fails += 1
                print(f"FAIL {name}: {e}")
    print("\nviolations on our USDC series:")
    for x in check_series(_load("our_USDC_1d.csv")):
        print("  ", x)
    sys.exit(1 if fails else 0)
