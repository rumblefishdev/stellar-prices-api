#!/usr/bin/env node
// Endpoint conformance suite for task 0120 (SCF M2, Tranche 2 AC 1).
//
// Exercises all 7 route groups for the 20 assets fixed in
// conformance-assets.json against the deployed production API, validates
// every response (including errors) against the live OpenAPI spec served at
// /api-docs-json, and layers sanity assertions the schema cannot express
// (sentinels, OHLCV invariants, pagination exhaustiveness, batch-vs-single
// agreement).
//
// Usage:
//   API_KEY=… BASE_URL=… node tools/scripts/conformance-0120.mjs
// or with the repo convention .env.local (API_KEY/BASE_URL) at the repo root.
//
// Output: markdown summary on stdout + a JSON report (citable evidence for
// task 0128) written next to the CWD as conformance-0120-report-<ts>.json.
// Exit code 1 if any check fails.

import { readFileSync, writeFileSync, existsSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import Ajv2020 from 'ajv/dist/2020.js';
import addFormats from 'ajv-formats';

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = join(HERE, '..', '..');

// ---------- env ----------
if (!process.env.API_KEY || !process.env.BASE_URL) {
  const envFile = join(REPO_ROOT, '.env.local');
  if (existsSync(envFile)) {
    for (const line of readFileSync(envFile, 'utf8').split('\n')) {
      const m = line.match(/^([A-Z_]+)=(.*)$/);
      if (m && !process.env[m[1]]) process.env[m[1]] = m[2];
    }
  }
}
const { API_KEY, BASE_URL } = process.env;
if (!API_KEY || !BASE_URL) {
  console.error('API_KEY and BASE_URL required (env or .env.local)');
  process.exit(2);
}

// ---------- paced, retrying client (free plan: 1 rps, burst 5) ----------
const PACE_MS = 1100;
let lastCall = 0;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
async function api(path, opts = {}) {
  const wait = lastCall + PACE_MS - Date.now();
  if (wait > 0) await sleep(wait);
  for (let attempt = 1; ; attempt++) {
    lastCall = Date.now();
    const res = await fetch(BASE_URL + path, {
      ...opts,
      headers: { 'x-api-key': API_KEY, ...(opts.headers || {}) },
    });
    if (res.status === 429 && attempt <= 3) {
      await sleep(3000 * attempt);
      continue;
    }
    const text = await res.text();
    let json = null;
    try {
      json = JSON.parse(text);
    } catch {
      // Non-JSON body (gateway HTML, empty 204) — `text` is kept for the report.
    }
    return { status: res.status, json, text };
  }
}

// ---------- report ----------
const checks = [];
function record(group, asset, name, ok, detail = '') {
  checks.push({ group, asset, name, status: ok ? 'pass' : 'fail', detail });
  if (!ok)
    console.log(
      `  FAIL  [${group}] ${asset} — ${name}${detail ? `: ${detail}` : ''}`,
    );
}
function skip(group, asset, name, detail) {
  checks.push({ group, asset, name, status: 'skip', detail });
}

// ---------- schema machinery ----------
const ajv = new Ajv2020({ strict: false, allErrors: true });
addFormats(ajv);
let spec;

function responseSchemaRef(pathTemplate, method, status) {
  const node =
    spec.paths?.[pathTemplate]?.[method]?.responses?.[String(status)]
      ?.content?.['application/json']?.schema;
  if (!node) return null;
  if (node.$ref) return 'spec' + node.$ref; // "#/components/…" -> "spec#/components/…"
  return null; // utoipa always emits $refs for named schemas; anything else is a spec smell
}

function validateAgainstSpec(group, asset, pathTemplate, method, status, body) {
  const ref = responseSchemaRef(pathTemplate, method, status);
  if (!ref) {
    record(
      group,
      asset,
      `spec has schema for ${method.toUpperCase()} ${pathTemplate} ${status}`,
      false,
    );
    return false;
  }
  const ok = ajv.validate({ $ref: ref }, body);
  record(
    group,
    asset,
    `schema-valid (${status})`,
    ok,
    ok ? '' : ajv.errorsText(ajv.errors, { separator: '; ' }).slice(0, 300),
  );
  return ok;
}

// Resolve $refs within the spec document (shallow, on demand).
function deref(node) {
  while (node && node.$ref) {
    const parts = node.$ref.replace(/^#\//, '').split('/');
    node = parts.reduce(
      (acc, p) => acc?.[p.replace(/~1/g, '/').replace(/~0/g, '~')],
      spec,
    );
  }
  return node;
}

// Strict pass beyond `required`: every documented property must be present.
// The AC reads "no documented field is absent", which is stronger than the
// schema's required-list. Nullable props are exempt — the API omits them when
// null by design (e.g. OhlcvResponse.backfill_note, documented as conditional).
function allPropsPresent(group, asset, schemaName, body) {
  const schema = deref(
    spec.components.schemas[schemaName]
      ? { $ref: `#/components/schemas/${schemaName}` }
      : null,
  );
  if (!schema?.properties) return;
  const missing = Object.keys(schema.properties).filter(
    (k) =>
      !(k in body) &&
      ![].concat(deref(schema.properties[k])?.type ?? []).includes('null'),
  );
  record(
    group,
    asset,
    `all documented fields present (${schemaName})`,
    missing.length === 0,
    missing.length ? `missing: ${missing.join(',')}` : '',
  );
}

const NUM_RE = /^-?\d+(\.\d+)?$/;

// ---------- OHLCV bucket classification (task 0230) ----------
// ADR 0011 §5: a bucket that traded but cannot yet be priced is returned
// PRESENT with null price fields, keeping `volume_base` and `trade_count` real.
// That is the contract. Asserting "every price field is a decimal string on
// every bucket" therefore made the suite's verdict depend on how far enrichment
// had caught up when it ran — USDCAllow, VELO and SHX failed at 13:26 and
// passed at 13:38 on 2026-08-27 with unchanged code. A conformance report whose
// result moves with a background worker cannot be cited as evidence, which is
// the whole reason task 0128 wants one.
const PRICE_FIELDS = ['open', 'high', 'low', 'close', 'vwap'];
const VOLUME_FIELDS = ['volume_base', 'volume_quote_usd'];

// 'priced' — every price field present. 'unpriced' — every price field null,
// the documented not-yet-priceable state. 'mixed' — some but not all, which is
// genuinely inconsistent and must still fail.
function bucketKind(c) {
  const nulls = PRICE_FIELDS.filter((f) => c[f] === null).length;
  if (nulls === 0) return 'priced';
  if (nulls === PRICE_FIELDS.length) return 'unpriced';
  return 'mixed';
}
function numericString(group, asset, field, value, { nonzero = false } = {}) {
  const ok =
    typeof value === 'string' &&
    NUM_RE.test(value) &&
    Number.isFinite(Number(value));
  record(
    group,
    asset,
    `${field} is a parseable decimal string`,
    ok,
    ok ? '' : JSON.stringify(value)?.slice(0, 80),
  );
  if (ok && nonzero) {
    record(
      group,
      asset,
      `${field} is not the zero sentinel`,
      Number(value) !== 0,
    );
  }
}

// ---------- report writer ----------
// Also the bail-out path: every early exit has to leave a report behind, or a
// failed run produces no evidence at all.
function finish() {
  const counts = { pass: 0, fail: 0, skip: 0 };
  for (const c of checks) counts[c.status]++;
  const report = {
    task: '0120',
    run_at: startedAt.toISOString(),
    duration_s: Math.round((Date.now() - startedAt.getTime()) / 1000),
    base_url: BASE_URL,
    spec_version: spec?.info?.version,
    asset_list: {
      file: 'tools/scripts/conformance-assets.json',
      derived_at: assetsFile.derived_at,
      count: ASSETS.length,
    },
    summary: counts,
    checks,
  };
  const out = `conformance-0120-report-${startedAt.toISOString().replace(/[:.]/g, '').slice(0, 15)}.json`;
  writeFileSync(out, JSON.stringify(report, null, 2));

  console.log(
    `\n# Summary: ${counts.pass} pass, ${counts.fail} fail, ${counts.skip} skip`,
  );
  const failGroups = {};
  for (const c of checks)
    if (c.status === 'fail')
      failGroups[c.group] = (failGroups[c.group] || 0) + 1;
  for (const [g, n] of Object.entries(failGroups))
    console.log(`  ${g}: ${n} failing`);
  console.log(`report: ${out}`);
  process.exit(counts.fail ? 1 : 0);
}

// ---------- suite ----------
const GRANULARITY_MS = {
  '1m': 60e3,
  '15m': 900e3,
  '1h': 3600e3,
  '4h': 14400e3,
  '1d': 86400e3,
};
// The tip is stamped by the producer's clock, not ours — a small negative age
// is skew between the two hosts, not a stale-data defect.
const CLOCK_SKEW_MS = 300e3;
const startedAt = new Date();

const assetsFile = JSON.parse(
  readFileSync(join(HERE, 'conformance-assets.json'), 'utf8'),
);
const ASSETS = assetsFile.assets;

console.log(`# Conformance 0120 — ${startedAt.toISOString()}`);
console.log(`base: ${BASE_URL}, assets: ${ASSETS.length}\n`);

// Group 0: the spec itself (also the validator input).
{
  const r = await api('/api-docs-json');
  record(
    'spec',
    '-',
    'GET /api-docs-json returns 200',
    r.status === 200,
    `status ${r.status}`,
  );
  // Nothing downstream can run without a usable spec — bail before ajv sees it,
  // and leave a report behind rather than dying on an unhandled TypeError.
  const usable =
    r.status === 200 && r.json && typeof r.json === 'object' && r.json.paths;
  record(
    'spec',
    '-',
    'spec body is a usable OpenAPI document',
    !!usable,
    usable ? '' : r.text.slice(0, 200),
  );
  if (!usable) {
    console.error('\nNo usable spec — cannot validate anything. Stopping.');
    finish();
  }
  spec = r.json;
  ajv.addSchema(spec, 'spec');
  record(
    'spec',
    '-',
    'spec declares OpenAPI 3.1',
    String(spec.openapi || '').startsWith('3.1'),
    spec.openapi,
  );
}

const singlePrices = new Map(); // id -> price body (for the batch comparison)

// Groups 1–4 + oracles, per asset.
for (const a of ASSETS) {
  const enc = encodeURIComponent(a.id);
  console.log(`## ${a.code || a.id.slice(0, 8)} (${a.form})`);

  // GET /v1/assets/{id} — detail
  {
    const r = await api(`/v1/assets/${enc}`);
    record(
      'detail',
      a.id,
      'returns 200',
      r.status === 200,
      `status ${r.status}`,
    );
    if (
      r.status === 200 &&
      validateAgainstSpec(
        'detail',
        a.id,
        '/v1/assets/{asset_identifier}',
        'get',
        200,
        r.json,
      )
    ) {
      allPropsPresent('detail', a.id, 'AssetDetail', r.json);
      record(
        'detail',
        a.id,
        'identity echoes the requested asset',
        r.json.asset === a.id,
      );
      record(
        'detail',
        a.id,
        'code matches the fixed list',
        r.json.code === a.code,
        `got ${JSON.stringify(r.json.code)}`,
      );
      record('detail', a.id, 'is_active', r.json.is_active === true);
    } else if (r.json) {
      validateAgainstSpec(
        'detail',
        a.id,
        '/v1/assets/{asset_identifier}',
        'get',
        r.status,
        r.json,
      );
    }
  }

  // GET /v1/assets/{id}/price
  {
    const r = await api(`/v1/assets/${enc}/price`);
    record(
      'price',
      a.id,
      'returns 200',
      r.status === 200,
      `status ${r.status}`,
    );
    if (
      r.status === 200 &&
      validateAgainstSpec(
        'price',
        a.id,
        '/v1/assets/{asset_identifier}/price',
        'get',
        200,
        r.json,
      )
    ) {
      allPropsPresent('price', a.id, 'PriceResponse', r.json);
      singlePrices.set(a.id, r.json);
      // `method` (task 0178, on the wire since 2026-09-01) says how price_usd
      // was arrived at, and it decides which derived columns carry a real value
      // and which carry their documented sentinel:
      //   traded — a real aggregate of this asset's own candles.
      //   oracle — a rate for an asset that never trades as a base leg, so it
      //            has no candles of its own; `vwap_24h "0"` and `sources {}`
      //            are the CORRECT answers, not missing work.
      //   ""     — the unavailable sentinel, paired with `price_usd "0"`.
      // Asserting "never zero" unconditionally read a DECIDED sentinel as a
      // PENDING defect, which is why this criterion could never go green.
      const method = r.json.method;
      const traded = method === 'traded';
      record(
        'price',
        a.id,
        'method is a documented value',
        ['traded', 'oracle', ''].includes(method),
        JSON.stringify(method),
      );
      // Assert the pairing in both directions. This holds whichever state the
      // asset is in, so it does not move with the market.
      record(
        'price',
        a.id,
        'price_usd is the zero sentinel exactly when method is ""',
        (Number(r.json.price_usd) === 0) === (method === ''),
        `price_usd=${r.json.price_usd} method=${JSON.stringify(method)}`,
      );
      numericString('price', a.id, 'price_usd', r.json.price_usd, {
        nonzero: method !== '',
      });
      numericString('price', a.id, 'vwap_24h', r.json.vwap_24h, {
        nonzero: traded,
      });
      // Parse-only, never non-zero. Runbook 0072: price_xlm / change_24h_pct
      // are legitimately zero on an un-enriched tip. volume_24h_usd is the same
      // shape of false alarm — a tracked asset can genuinely go a day without a
      // trade, and 0128 re-runs this suite on whatever the market did that day.
      for (const f of ['price_xlm', 'change_24h_pct', 'volume_24h_usd'])
        numericString('price', a.id, f, r.json[f]);
      const srcs = Object.keys(r.json.sources || {});
      // A per-source breakdown exists only where the price came from this
      // asset's own traded candles. For `oracle` and `""` the documented
      // answer is `{}` (dto.rs: "no source qualified — the exotic-quote and
      // all-below-threshold cases, not an error").
      record(
        'price',
        a.id,
        traded
          ? 'sources is a non-empty object'
          : `sources is the documented {} for method ${JSON.stringify(method)}`,
        traded ? srcs.length > 0 : srcs.length === 0,
        `${srcs.length} sources`,
      );
      for (const s of srcs) {
        numericString(
          'price',
          a.id,
          `sources.${s}.price`,
          r.json.sources[s].price,
          { nonzero: true },
        );
        numericString(
          'price',
          a.id,
          `sources.${s}.volume_24h`,
          r.json.sources[s].volume_24h,
        );
      }
      const age = Date.now() - Date.parse(r.json.updated_at);
      record(
        'price',
        a.id,
        'updated_at within 24h',
        age > -CLOCK_SKEW_MS && age < 86400e3,
        r.json.updated_at,
      );
    } else if (r.json) {
      validateAgainstSpec(
        'price',
        a.id,
        '/v1/assets/{asset_identifier}/price',
        'get',
        r.status,
        r.json,
      );
    }
  }

  // GET /v1/assets/{id}/ohlcv — two granularities, explicit windows
  // (finding 4: the default window is narrower than `limit` implies).
  for (const [gran, days] of [
    ['1h', 7],
    ['1d', 30],
  ]) {
    const step = GRANULARITY_MS[gran];
    const end = new Date(Math.floor(Date.now() / step) * step);
    const start = new Date(end.getTime() - days * 86400e3);
    const q = `granularity=${gran}&start=${start.toISOString()}&end=${end.toISOString()}`;
    const r = await api(`/v1/assets/${enc}/ohlcv?${q}`);
    const tag = `ohlcv:${gran}`;
    record(tag, a.id, 'returns 200', r.status === 200, `status ${r.status}`);
    if (
      r.status === 200 &&
      validateAgainstSpec(
        tag,
        a.id,
        '/v1/assets/{asset_identifier}/ohlcv',
        'get',
        200,
        r.json,
      )
    ) {
      allPropsPresent(tag, a.id, 'OhlcvResponse', r.json);
      const data = r.json.data || [];
      record(
        tag,
        a.id,
        'window is non-empty for a liquid asset',
        data.length > 0,
        `${data.length} buckets`,
      );
      let ordered = true,
        aligned = true,
        dup = false,
        ohlc = true,
        numeric = true,
        volumes = true,
        unpricedShape = true,
        mixed = 0,
        unpriced = 0;
      let prev = -Infinity;
      for (const c of data) {
        const ts = Date.parse(c.timestamp);
        if (ts <= prev) ordered = false;
        if (ts === prev) dup = true;
        if (ts % step !== 0) aligned = false;
        prev = ts;
        // Volume is real on every bucket, priced or not — that is what
        // distinguishes "traded, not yet priceable" from "never traded".
        for (const f of VOLUME_FIELDS)
          if (!(typeof c[f] === 'string' && NUM_RE.test(c[f]))) volumes = false;
        const kind = bucketKind(c);
        if (kind === 'unpriced') {
          unpriced++;
          // ADR 0011 §5: price fields null, `method` and `derived` null with
          // them, volume and trade_count still real.
          if (
            c.method !== null ||
            c.derived !== null ||
            !Number.isFinite(Number(c.trade_count))
          )
            unpricedShape = false;
          continue;
        }
        if (kind === 'mixed') {
          mixed++;
          numeric = false;
          continue;
        }
        const [o, h, l, cl] = [c.open, c.high, c.low, c.close].map(Number);
        for (const f of PRICE_FIELDS)
          if (!(typeof c[f] === 'string' && NUM_RE.test(c[f]))) numeric = false;
        if (!(l <= Math.min(o, cl) && Math.max(o, cl) <= h)) ohlc = false;
      }
      if (data.length) {
        record(tag, a.id, 'timestamps strictly increasing', ordered && !dup);
        record(tag, a.id, `timestamps aligned to ${gran}`, aligned);
        record(
          tag,
          a.id,
          'low <= open,close <= high on every priced bucket',
          ohlc,
        );
        record(
          tag,
          a.id,
          'all priced OHLCV values are decimal strings',
          numeric,
          mixed ? `${mixed} bucket(s) part-priced` : '',
        );
        record(
          tag,
          a.id,
          'volume fields are decimal strings on every bucket',
          volumes,
        );
        record(
          tag,
          a.id,
          'unpriced buckets carry the ADR 0011 §5 shape (method/derived null, trade_count real)',
          unpricedShape,
          `${unpriced} of ${data.length} buckets unpriced`,
        );
        // Both window ends are inclusive (measured 2026-08-19: a 5-day
        // start/end range returns 6 buckets). Undocumented in §4 — flagged as
        // a docs gap by the task, but the check follows the implementation.
        const inWindow = data.every(
          (c) =>
            Date.parse(c.timestamp) >= start.getTime() &&
            Date.parse(c.timestamp) <= end.getTime(),
        );
        record(
          tag,
          a.id,
          'all buckets inside the requested window (inclusive ends)',
          inWindow,
        );
      }
    } else if (r.json) {
      validateAgainstSpec(
        tag,
        a.id,
        '/v1/assets/{asset_identifier}/ohlcv',
        'get',
        r.status,
        r.json,
      );
    }
  }

  // GET /v1/oracles/{id}
  {
    const r = await api(`/v1/oracles/${enc}`);
    record(
      'oracles',
      a.id,
      'returns 200 or documented 404',
      [200, 404].includes(r.status),
      `status ${r.status}`,
    );
    if (r.json)
      validateAgainstSpec(
        'oracles',
        a.id,
        '/v1/oracles/{asset_identifier}',
        'get',
        r.status,
        r.json,
      );
    if (r.status === 200)
      allPropsPresent('oracles', a.id, 'OraclesResponse', r.json);
  }
}

// Group 5: GET /v1/assets pagination — walk to exhaustion.
{
  console.log(`## pagination walk`);
  const seen = new Map(); // identity key -> count
  // Resolve once, with the same guard validateAgainstSpec applies: a bare
  // {$ref: null} makes ajv throw and kills the walk.
  const listRef = responseSchemaRef('/v1/assets', 'get', 200);
  record('list', '-', 'spec has schema for GET /v1/assets 200', !!listRef);
  let cursor = null,
    pages = 0,
    lastHasMore = null,
    schemaOk = true;
  do {
    const q = cursor
      ? `limit=200&cursor=${encodeURIComponent(cursor)}`
      : 'limit=200';
    const r = await api(`/v1/assets?${q}`);
    if (r.status !== 200) {
      record(
        'list',
        '-',
        `page ${pages + 1} returns 200`,
        false,
        `status ${r.status}`,
      );
      break;
    }
    if (listRef && !ajv.validate({ $ref: listRef }, r.json)) schemaOk = false;
    // Do not walk a shape validation just rejected.
    if (!Array.isArray(r.json.data)) {
      record(
        'list',
        '-',
        `page ${pages + 1} carries a data array`,
        false,
        `got ${typeof r.json.data}`,
      );
      break;
    }
    for (const item of r.json.data) {
      const key = `${item.asset_code}|${item.issuer_address}|${item.contract_address}`;
      seen.set(key, (seen.get(key) || 0) + 1);
    }
    record(
      'list',
      '-',
      `page ${pages + 1}: has_more consistent with cursor`,
      r.json.has_more === (r.json.cursor != null),
      `has_more=${r.json.has_more} cursor=${r.json.cursor == null ? 'null' : 'set'}`,
    );
    cursor = r.json.cursor;
    lastHasMore = r.json.has_more;
    pages++;
  } while (cursor && pages < 100);
  // Only claim this when a schema was actually applied — otherwise the pass
  // would mean "never checked".
  if (listRef)
    record('list', '-', 'every page validates against the schema', schemaOk);
  record(
    'list',
    '-',
    'walk terminates (has_more=false on last page)',
    lastHasMore === false && pages < 100,
    `${pages} pages`,
  );
  const dups = [...seen.entries()].filter(([, n]) => n > 1);
  record(
    'list',
    '-',
    'no asset appears twice across the walk',
    dups.length === 0,
    dups
      .slice(0, 5)
      .map(([k, n]) => `${k}×${n}`)
      .join(', '),
  );
  record(
    'list',
    '-',
    'walk yields a plausible asset count (>=200)',
    seen.size >= 200,
    `${seen.size} assets`,
  );
  console.log(`  ${pages} pages, ${seen.size} distinct assets`);
}

// ---------- batch/single snapshot alignment ----------
// `current_prices` refreshes every minute and the suite paces at ~1.1 s, so a
// single taken in one refresh window and the bulk batch taken in the next
// disagree by construction. /price is gateway-cached for 10 s while batch is
// uncached, so a "fresh" single inside that window is the same cached body —
// hence the wait before each retry.
const ALIGN_ATTEMPTS = 3;
const PRICE_CACHE_TTL_MS = 10_000;
const SNAPSHOT_FIELDS = [
  'price_usd',
  'price_xlm',
  'vwap_24h',
  'volume_24h_usd',
];
const sameSnapshot = (x, y) => SNAPSHOT_FIELDS.every((f) => x[f] === y[f]);

async function alignedPair(id) {
  for (let attempt = 1; attempt <= ALIGN_ATTEMPTS; attempt++) {
    if (attempt > 1) await sleep(PRICE_CACHE_TTL_MS + 1000);
    const s = await api(`/v1/assets/${encodeURIComponent(id)}/price`);
    const bt = await api(`/v1/prices/batch`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ assets: [id] }),
    });
    if (s.status !== 200 || bt.status !== 200) continue;
    const bp = (bt.json.prices || []).find((p) => p.asset === id);
    if (bp && bp.updated_at === s.json.updated_at)
      return { single: s.json, batch: bp, attempt };
  }
  return null;
}

// Group 6: POST /v1/prices/batch vs the per-asset singles.
{
  console.log(`## batch vs single`);
  const ids = ASSETS.map((a) => a.id);
  const r = await api(`/v1/prices/batch`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ assets: ids }),
  });
  record('batch', '-', 'returns 200', r.status === 200, `status ${r.status}`);
  if (
    r.status === 200 &&
    validateAgainstSpec('batch', '-', '/v1/prices/batch', 'post', 200, r.json)
  ) {
    const returned = new Map(r.json.prices.map((p) => [p.asset, p]));
    const partition = [...returned.keys(), ...r.json.not_found].sort();
    record(
      'batch',
      '-',
      'prices + not_found partition the request exactly',
      JSON.stringify(partition) === JSON.stringify([...ids].sort()),
      `got ${returned.size} prices + ${r.json.not_found.length} not_found of ${ids.length}`,
    );
    for (const [id, single] of singlePrices) {
      const b = returned.get(id);
      if (!b) {
        record(
          'batch',
          id,
          'asset with a single price is present in batch',
          false,
        );
        continue;
      }
      if (b.updated_at === single.updated_at) {
        record(
          'batch',
          id,
          'batch equals single at the same timestamp',
          sameSnapshot(b, single),
        );
      } else {
        // The old path re-fetched the SINGLE and compared it to the bulk batch
        // taken minutes earlier. That cannot converge: the bulk batch is fixed
        // in the past while a fresh single only moves forward, so 11 of 19
        // assets skipped rather than being checked. Re-take BOTH instead, back
        // to back, and compare that pair.
        const pair = await alignedPair(id);
        if (pair) {
          record(
            'batch',
            id,
            'batch equals single at the same timestamp',
            sameSnapshot(pair.batch, pair.single),
            `re-taken as a pair, attempt ${pair.attempt}`,
          );
        } else {
          skip(
            'batch',
            id,
            'batch/single timestamps never aligned',
            `${ALIGN_ATTEMPTS} paired re-takes all straddled a refresh`,
          );
        }
      }
    }
  }
}

// Group 7: GET /v1/backfill/status.
{
  console.log(`## backfill status`);
  const r = await api(`/v1/backfill/status`);
  record(
    'backfill',
    '-',
    'returns 200',
    r.status === 200,
    `status ${r.status}`,
  );
  if (
    r.status === 200 &&
    validateAgainstSpec(
      'backfill',
      '-',
      '/v1/backfill/status',
      'get',
      200,
      r.json,
    )
  ) {
    allPropsPresent('backfill', '-', 'BackfillStatus', r.json);
  }
}

// ---------- summary ----------
finish();
