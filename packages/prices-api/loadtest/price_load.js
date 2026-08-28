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
//   ≥2000(wide)|      30,000 |        100 %    | the real data path (CH round trip)
//
// ⚠️ The wide regime needs pool ≫ RATE × TTL, not "a big number". Selection is
// deterministic round-robin, so a pool of exactly RATE × TTL (1000 at 100 req/s
// and a 10 s TTL) returns to each asset every 10.0 s — precisely the TTL, making
// hit-vs-miss a timing coin-flip and the p95 neither number. 2000 gives 2x
// margin; scale it with RATE if you change the rate.
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
// pollutes p95. The run opens with a warmup phase tagged `phase:warmup` and
// EXCLUDED from every threshold. Cite the `phase:main` window; read cold-start
// incidence from Lambda's InitDuration in CloudWatch for the same window.
//
// The warmup runs at the FULL rate, briefly — what needs warming is Lambda
// *concurrency*, and concurrency is set by the rate, not by the duration. An
// earlier version warmed at RATE/10, which sustains roughly one execution
// environment, so ~9 of the ~10 the main phase needs still cold-started inside
// the measured window (PR #234 review). Honest scale: ~9 cold starts in 30 000
// samples would not move p95 either way, so this is about the phase doing what
// its name says, not about a threat to the number.
//
// `-e WARMUP=0` (or `0s`) drops the phase entirely rather than declaring a
// zero-duration scenario, which k6 rejects at config validation.

import http from 'k6/http';
import { check } from 'k6';
import exec from 'k6/execution';
import { SharedArray } from 'k6/data';

const BASE_URL = __ENV.BASE_URL || 'http://localhost:8080';
const API_KEY = __ENV.API_KEY || '';
const RATE = Number(__ENV.RATE || 100);
const WARMUP = __ENV.WARMUP || '30s';
const DURATION = __ENV.DURATION || '5m';

// Asset pool. ASSET pins a single id (cache-dominated); otherwise the pool is
// read from a JSON file — the 20-asset conformance list by default, so 0121 and
// 0120 measure the same assets.
//
// SharedArray, not a bare `open()`: k6 runs init once PER VU and copies plain
// init-context data into each, while SharedArray parses once for the whole run.
// Irrelevant at 20 assets, but the wide regime wants thousands across up to 400
// VUs, and the failure mode there is indirect — a memory-bound generator drops
// iterations, `dropped_iterations` fails correctly, and the operator is sent to
// raise MAX_VUS, which makes it worse (PR #234 review).
const POOL = new SharedArray('pool', () => {
  if (__ENV.ASSET) return [__ENV.ASSET];
  const raw = JSON.parse(open(__ENV.ASSETS || '../../../tools/scripts/conformance-assets.json'));
  const list = Array.isArray(raw) ? raw : raw.assets;
  return list.map((a) => (typeof a === 'string' ? a : a.id));
});

// `0`/`0s`/empty drops the phase rather than declaring a zero-duration scenario.
const WARMUP_ON = !!WARMUP && WARMUP !== '0' && WARMUP !== '0s';
const VU_POOL = {
  preAllocatedVUs: Number(__ENV.VUS || 50),
  maxVUs: Number(__ENV.MAX_VUS || 200),
};

export const options = {
  // The probe below is one request per pool asset. k6's default setupTimeout is
  // 60 s, and at the measured ~83 ms per probe a sequential walk of the wide
  // pool this README recommends would abort the run before a single measured
  // request is sent. Probing is batched (see setup) and the ceiling is raised.
  setupTimeout: __ENV.SETUP_TIMEOUT || '180s',
  scenarios: {
    // Excluded from thresholds — its job is to have containers already warm.
    ...(WARMUP_ON
      ? {
          warmup: {
            executor: 'constant-arrival-rate',
            rate: RATE,
            timeUnit: '1s',
            duration: WARMUP,
            ...VU_POOL,
            tags: { phase: 'warmup' },
          },
        }
      : {}),
    main: {
      executor: 'constant-arrival-rate',
      rate: RATE,
      timeUnit: '1s',
      duration: DURATION,
      ...(WARMUP_ON ? { startTime: WARMUP } : {}),
      ...VU_POOL,
      tags: { phase: 'main' },
    },
  },
  thresholds: {
    'http_req_duration{phase:main}': ['p(95)<200'],
    'http_req_failed{phase:main}': ['rate<0.001'],
    // k6 check results do NOT affect the exit code — only thresholds do. Without
    // this line the body validation below is decorative: a regression serving
    // `200 {"price_usd": null}` on every request passes every other threshold
    // and the harness certifies the SLO against a broken endpoint (PR #234
    // review). Scoped to main like the rest, so warmup stays excluded.
    'checks{phase:main}': ['rate>0.99'],
    // Dropped iterations mean k6 could not keep the offered rate — the run did
    // NOT sustain 100 req/s and its p95 is not the AC's number. Scoped to main:
    // a drop during warmup is containers scaling, which is that phase's whole
    // purpose, and must not be reported as "did not sustain 100 req/s".
    'dropped_iterations{phase:main}': ['count<1'],
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
  const missing = []; // 404 — no current-price row; drop from the pool.
  const transient = []; // 429/5xx — says nothing about the asset. Fail loudly.

  // Batched, not sequential: at ~83 ms a probe, a 2000-asset pool would take
  // ~166 s serially and blow even the raised setupTimeout above.
  const BATCH = Number(__ENV.PROBE_BATCH || 10);
  for (let i = 0; i < POOL.length; i += BATCH) {
    const slice = [];
    for (let j = i; j < Math.min(i + BATCH, POOL.length); j++) slice.push(POOL[j]);
    const responses = http.batch(
      slice.map((asset) => [
        'GET',
        `${BASE_URL}/v1/assets/${encodeURIComponent(asset)}/price`,
        null,
        { ...PARAMS, tags: { phase: 'probe' } },
      ]),
    );
    responses.forEach((res, k) => {
      if (res.status === 200) live.push(slice[k]);
      else if (res.status === 404) missing.push(`${slice[k]} → 404`);
      else transient.push(`${slice[k]} → ${res.status}`);
    });
  }

  // A 404 is a property of the asset (task 0178's canonical USDC is exactly
  // this), so dropping it is right — one dead asset in twenty would otherwise
  // put a permanent 5 % floor under an error rate the AC caps at 0.1 %.
  if (missing.length) {
    console.warn(`pool: dropped ${missing.length}/${POOL.length} asset(s) with no price row: ${missing.join(', ')}`);
  }
  // Anything else is a property of the RUN, not the asset. Silently shrinking
  // the pool here reports "the data is dead" when the real cause is usually the
  // key: `pricing-api-free-production` is capped at 1 req/s, so a probe on that
  // plan is throttled on nearly every asset (PR #234 review).
  if (transient.length) {
    exec.test.abort(
      `pool: ${transient.length}/${POOL.length} probe(s) failed with a non-404 — this is about the run, not the assets. ` +
        `429 means the key's usage plan cannot carry the probe (see the README on prices-production-loadtest-plan); ` +
        `5xx means the API is unhealthy. First failures: ${transient.slice(0, 5).join(', ')}`,
    );
  }
  if (!live.length) {
    exec.test.abort(`pool: no asset answered 200 out of ${POOL.length} probed — nothing to measure`);
  }
  console.log(`pool: ${live.length} asset(s) under test`);
  return { pool: live };
}

// No `X-Request-Id` here on purpose. An earlier revision stamped one and
// described it as joinable to ClickHouse `system.query_log.log_comment` — the
// API neither reads that header nor sets `log_comment` anywhere in the repo, so
// the header was inert and the claim would have sent the next reader hunting
// for a join that cannot be made (PR #234 review). Worth knowing before anyone
// adds it back: in the 20-asset regime ~98 % of main-phase requests are served
// by the gateway cache and never reach the Lambda, so even once plumbed the
// join would only ever cover the misses.
export default function (data) {
  const pool = data.pool;
  const asset = pool[exec.scenario.iterationInTest % pool.length];
  const res = http.get(`${BASE_URL}/v1/assets/${encodeURIComponent(asset)}/price`, PARAMS);
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
