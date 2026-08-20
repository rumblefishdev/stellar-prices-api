// k6 load test for the §9 SLO and Tranche 2 AC 2 (task 0121):
//   100 req/s sustained for 5 minutes on GET /assets/{id}/price
//   → p95 latency < 200 ms, error rate < 0.1%
//
// Run against a deployed stage (authoritative) or the local server (approximate
// — no gateway cache / no Lambda cold start, so a lower bound on prod p95):
//
//   k6 run packages/prices-api/loadtest/price_load.js \
//     -e BASE_URL=https://<api>/<stage> -e API_KEY=<key>
//
// Knobs (env): RATE (req/s, default 100), DURATION (default 5m), WARMUP
// (default 30s), ASSET (single-asset mode), ASSETS (path to an id pool),
// API_KEY, VUS, MAX_VUS. k6 exits non-zero if a threshold is breached.
//
// ── Why the asset pool decides what you are measuring ───────────────────────
// The gateway caches /price for 10 s keyed on the PATH ONLY (`addGet(price,
// [PATH_ID])` in api-gateway-stack.ts) — no query parameter can bust it, so the
// number of DISTINCT assets in the pool is the only lever on the hit rate.
// Over a 300 s run each asset can miss at most 300/10 = 30 times, so:
//
//   pool size |  max misses  | of 30k requests | what the p95 mostly measures
//   ----------|--------------|-----------------|------------------------------
//   1  (hot)  |          30  |          0.1 %  | the gateway cache
//   20 (spread)|        600  |          2 %    | the AC scenario, cache-dominated
//   1000+(wide)|     ≥30,000 |        100 %    | the real data path (CH round trip)
//
// There is no `X-Cache` header on this API (verified 2026-08-20), so hit/miss
// percentiles cannot be tagged per request — run the regimes separately and
// report which one each number came from. Uniform sampling across the pool is
// the conservative choice: it never warms a hot key the way real traffic would,
// so `wide` is a worst case rather than a typical one. Say so in the report
// instead of inventing a traffic distribution.
//
// ── Warmup ─────────────────────────────────────────────────────────────────
// A step-start at 100 req/s pays Lambda cold starts in the first seconds, which
// visibly pollutes p95 over 30k samples. The run therefore opens with a low-rate
// warmup phase, tagged `phase:warmup` and EXCLUDED from every threshold. Cite
// the `phase:main` window; read cold-start incidence from Lambda's InitDuration
// in CloudWatch for the same window.

import http from 'k6/http';
import { check } from 'k6';
import exec from 'k6/execution';

const BASE_URL = __ENV.BASE_URL || 'http://localhost:8080';
const API_KEY = __ENV.API_KEY || '';
const RATE = Number(__ENV.RATE || 100);
const WARMUP = __ENV.WARMUP || '30s';
const DURATION = __ENV.DURATION || '5m';

// Asset pool. ASSET pins a single id (cache-dominated); otherwise the pool is
// read from a JSON file — the 20-asset conformance list by default, so 0121 and
// 0120 measure the same assets.
function loadPool(path) {
  const raw = JSON.parse(open(path));
  const list = Array.isArray(raw) ? raw : raw.assets;
  return list.map((a) => (typeof a === 'string' ? a : a.id));
}
const POOL = __ENV.ASSET
  ? [__ENV.ASSET]
  : loadPool(__ENV.ASSETS || '../../../tools/scripts/conformance-assets.json');

export const options = {
  scenarios: {
    // Excluded from thresholds — its job is to have containers already warm.
    warmup: {
      executor: 'constant-arrival-rate',
      rate: Math.max(1, Math.round(RATE / 10)),
      timeUnit: '1s',
      duration: WARMUP,
      preAllocatedVUs: Number(__ENV.VUS || 50),
      maxVUs: Number(__ENV.MAX_VUS || 200),
      tags: { phase: 'warmup' },
    },
    main: {
      executor: 'constant-arrival-rate',
      rate: RATE,
      timeUnit: '1s',
      duration: DURATION,
      startTime: WARMUP,
      preAllocatedVUs: Number(__ENV.VUS || 50),
      maxVUs: Number(__ENV.MAX_VUS || 200),
      tags: { phase: 'main' },
    },
  },
  thresholds: {
    'http_req_duration{phase:main}': ['p(95)<200'],
    'http_req_failed{phase:main}': ['rate<0.001'],
    // Dropped iterations mean k6 could not keep the offered rate — the run did
    // NOT sustain 100 req/s and its p95 is not the AC's number.
    dropped_iterations: ['count<1'],
  },
};

const PARAMS = {
  headers: {
    ...(API_KEY ? { 'x-api-key': API_KEY } : {}),
    // Managed WAF rulesets 403 a missing User-Agent; k6 sends one, this pins it.
    'User-Agent': 'stellar-prices-api-loadtest/0121 (k6)',
  },
  tags: { endpoint: 'price' },
  // Anything other than 200 is a failure. The default (status < 400) would let
  // a 204 or a challenge served as 2xx pass silently.
  responseCallback: http.expectedStatuses(200),
};

// Probe the pool once and keep only assets the API can actually serve. An asset
// with no current-price row answers 404 forever, so leaving one in the pool puts
// a floor under the error rate that has nothing to do with load: one dead asset
// in twenty is a permanent 5 %, against an AC of 0.1 %. Measured 2026-08-20 —
// canonical USDC is exactly this case (task 0178), so the default pool needs the
// probe to be usable at all. Whatever it drops is printed: put the list in the
// report rather than letting it vanish.
export function setup() {
  const live = [];
  const dropped = [];
  for (const asset of POOL) {
    const res = http.get(`${BASE_URL}/v1/assets/${encodeURIComponent(asset)}/price`, {
      ...PARAMS,
      tags: { phase: 'probe' },
    });
    (res.status === 200 ? live : dropped).push(res.status === 200 ? asset : `${asset} → ${res.status}`);
  }
  if (dropped.length) {
    console.warn(`pool: dropped ${dropped.length}/${POOL.length} unservable asset(s): ${dropped.join(', ')}`);
  }
  if (!live.length) {
    exec.test.abort('pool: no asset answered 200 — nothing to measure');
  }
  console.log(`pool: ${live.length} asset(s) under test`);
  return { pool: live };
}

export default function (data) {
  const pool = data.pool;
  const asset = pool[exec.scenario.iterationInTest % pool.length];
  const params = {
    ...PARAMS,
    headers: {
      ...PARAMS.headers,
      // Stamped into ClickHouse system.query_log.log_comment when the API runs
      // with request-id logging, so a slow request can be joined to its query.
      'X-Request-Id': `lt0121-${exec.scenario.iterationInTest}-${__VU}`,
    },
  };
  const res = http.get(`${BASE_URL}/v1/assets/${encodeURIComponent(asset)}/price`, params);
  check(res, {
    'status is 200': (r) => r.status === 200,
    // A body-read failure must not pass as a slow 200.
    'body parses with price_usd': (r) => {
      try {
        return typeof r.json('price_usd') === 'string';
      } catch (_) {
        return false;
      }
    },
  });
}
