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

import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import Ajv2020 from "ajv/dist/2020.js";
import addFormats from "ajv-formats";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = join(HERE, "..", "..");

// ---------- env ----------
if (!process.env.API_KEY || !process.env.BASE_URL) {
  const envFile = join(REPO_ROOT, ".env.local");
  if (existsSync(envFile)) {
    for (const line of readFileSync(envFile, "utf8").split("\n")) {
      const m = line.match(/^([A-Z_]+)=(.*)$/);
      if (m && !process.env[m[1]]) process.env[m[1]] = m[2];
    }
  }
}
const { API_KEY, BASE_URL } = process.env;
if (!API_KEY || !BASE_URL) {
  console.error("API_KEY and BASE_URL required (env or .env.local)");
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
      headers: { "x-api-key": API_KEY, ...(opts.headers || {}) },
    });
    if (res.status === 429 && attempt <= 3) {
      await sleep(3000 * attempt);
      continue;
    }
    const text = await res.text();
    let json = null;
    try {
      json = JSON.parse(text);
    } catch {}
    return { status: res.status, json, text };
  }
}

// ---------- report ----------
const checks = [];
function record(group, asset, name, ok, detail = "") {
  checks.push({ group, asset, name, status: ok ? "pass" : "fail", detail });
  if (!ok) console.log(`  FAIL  [${group}] ${asset} — ${name}${detail ? `: ${detail}` : ""}`);
}
function skip(group, asset, name, detail) {
  checks.push({ group, asset, name, status: "skip", detail });
}

// ---------- schema machinery ----------
const ajv = new Ajv2020({ strict: false, allErrors: true });
addFormats(ajv);
let spec;

function responseSchemaRef(pathTemplate, method, status) {
  const node =
    spec.paths?.[pathTemplate]?.[method]?.responses?.[String(status)]?.content?.[
      "application/json"
    ]?.schema;
  if (!node) return null;
  if (node.$ref) return "spec" + node.$ref; // "#/components/…" -> "spec#/components/…"
  return null; // utoipa always emits $refs for named schemas; anything else is a spec smell
}

function validateAgainstSpec(group, asset, pathTemplate, method, status, body) {
  const ref = responseSchemaRef(pathTemplate, method, status);
  if (!ref) {
    record(group, asset, `spec has schema for ${method.toUpperCase()} ${pathTemplate} ${status}`, false);
    return false;
  }
  const ok = ajv.validate({ $ref: ref }, body);
  record(
    group,
    asset,
    `schema-valid (${status})`,
    ok,
    ok ? "" : ajv.errorsText(ajv.errors, { separator: "; " }).slice(0, 300)
  );
  return ok;
}

// Resolve $refs within the spec document (shallow, on demand).
function deref(node) {
  while (node && node.$ref) {
    const parts = node.$ref.replace(/^#\//, "").split("/");
    node = parts.reduce((acc, p) => acc?.[p.replace(/~1/g, "/").replace(/~0/g, "~")], spec);
  }
  return node;
}

// Strict pass beyond `required`: every documented property must be present.
// The AC reads "no documented field is absent", which is stronger than the
// schema's required-list. Nullable props are exempt — the API omits them when
// null by design (e.g. OhlcvResponse.backfill_note, documented as conditional).
function allPropsPresent(group, asset, schemaName, body) {
  const schema = deref(spec.components.schemas[schemaName] ? { $ref: `#/components/schemas/${schemaName}` } : null);
  if (!schema?.properties) return;
  const missing = Object.keys(schema.properties).filter(
    (k) => !(k in body) && ![].concat(deref(schema.properties[k])?.type ?? []).includes("null")
  );
  record(group, asset, `all documented fields present (${schemaName})`, missing.length === 0,
    missing.length ? `missing: ${missing.join(",")}` : "");
}

const NUM_RE = /^-?\d+(\.\d+)?$/;
function numericString(group, asset, field, value, { nonzero = false } = {}) {
  const ok = typeof value === "string" && NUM_RE.test(value) && Number.isFinite(Number(value));
  record(group, asset, `${field} is a parseable decimal string`, ok, ok ? "" : JSON.stringify(value)?.slice(0, 80));
  if (ok && nonzero) {
    record(group, asset, `${field} is not the zero sentinel`, Number(value) !== 0);
  }
}

// ---------- suite ----------
const GRANULARITY_MS = { "1m": 60e3, "15m": 900e3, "1h": 3600e3, "4h": 14400e3, "1d": 86400e3 };
const startedAt = new Date();

const assetsFile = JSON.parse(readFileSync(join(HERE, "conformance-assets.json"), "utf8"));
const ASSETS = assetsFile.assets;

console.log(`# Conformance 0120 — ${startedAt.toISOString()}`);
console.log(`base: ${BASE_URL}, assets: ${ASSETS.length}\n`);

// Group 0: the spec itself (also the validator input).
{
  const r = await api("/api-docs-json");
  record("spec", "-", "GET /api-docs-json returns 200", r.status === 200, `status ${r.status}`);
  spec = r.json;
  ajv.addSchema(spec, "spec");
  record("spec", "-", "spec declares OpenAPI 3.1", String(spec.openapi || "").startsWith("3.1"), spec.openapi);
}

const singlePrices = new Map(); // id -> price body (for the batch comparison)

// Groups 1–4 + oracles, per asset.
for (const a of ASSETS) {
  const enc = encodeURIComponent(a.id);
  console.log(`## ${a.code || a.id.slice(0, 8)} (${a.form})`);

  // GET /v1/assets/{id} — detail
  {
    const r = await api(`/v1/assets/${enc}`);
    record("detail", a.id, "returns 200", r.status === 200, `status ${r.status}`);
    if (r.status === 200 && validateAgainstSpec("detail", a.id, "/v1/assets/{asset_identifier}", "get", 200, r.json)) {
      allPropsPresent("detail", a.id, "AssetDetail", r.json);
      record("detail", a.id, "identity echoes the requested asset", r.json.asset === a.id);
      record("detail", a.id, "code matches the fixed list", r.json.code === a.code,
        `got ${JSON.stringify(r.json.code)}`);
      record("detail", a.id, "is_active", r.json.is_active === true);
    } else if (r.json) {
      validateAgainstSpec("detail", a.id, "/v1/assets/{asset_identifier}", "get", r.status, r.json);
    }
  }

  // GET /v1/assets/{id}/price
  {
    const r = await api(`/v1/assets/${enc}/price`);
    record("price", a.id, "returns 200", r.status === 200, `status ${r.status}`);
    if (r.status === 200 && validateAgainstSpec("price", a.id, "/v1/assets/{asset_identifier}/price", "get", 200, r.json)) {
      allPropsPresent("price", a.id, "PriceResponse", r.json);
      singlePrices.set(a.id, r.json);
      for (const f of ["price_usd", "vwap_24h", "volume_24h_usd"])
        numericString("price", a.id, f, r.json[f], { nonzero: true });
      // Runbook 0072: price_xlm / change_24h_pct may legitimately be zero on an
      // un-enriched tip — assert parseability only, never non-zero.
      for (const f of ["price_xlm", "change_24h_pct"]) numericString("price", a.id, f, r.json[f]);
      const srcs = Object.keys(r.json.sources || {});
      record("price", a.id, "sources is a non-empty object", srcs.length > 0);
      for (const s of srcs) {
        numericString("price", a.id, `sources.${s}.price`, r.json.sources[s].price, { nonzero: true });
        numericString("price", a.id, `sources.${s}.volume_24h`, r.json.sources[s].volume_24h);
      }
      const age = Date.now() - Date.parse(r.json.updated_at);
      record("price", a.id, "updated_at within 24h", age >= 0 && age < 86400e3, r.json.updated_at);
    } else if (r.json) {
      validateAgainstSpec("price", a.id, "/v1/assets/{asset_identifier}/price", "get", r.status, r.json);
    }
  }

  // GET /v1/assets/{id}/ohlcv — two granularities, explicit windows
  // (finding 4: the default window is narrower than `limit` implies).
  for (const [gran, days] of [["1h", 7], ["1d", 30]]) {
    const step = GRANULARITY_MS[gran];
    const end = new Date(Math.floor(Date.now() / step) * step);
    const start = new Date(end.getTime() - days * 86400e3);
    const q = `granularity=${gran}&start=${start.toISOString()}&end=${end.toISOString()}`;
    const r = await api(`/v1/assets/${enc}/ohlcv?${q}`);
    const tag = `ohlcv:${gran}`;
    record(tag, a.id, "returns 200", r.status === 200, `status ${r.status}`);
    if (r.status === 200 && validateAgainstSpec(tag, a.id, "/v1/assets/{asset_identifier}/ohlcv", "get", 200, r.json)) {
      allPropsPresent(tag, a.id, "OhlcvResponse", r.json);
      const data = r.json.data || [];
      record(tag, a.id, "window is non-empty for a liquid asset", data.length > 0, `${data.length} buckets`);
      let ordered = true, aligned = true, dup = false, ohlc = true, numeric = true;
      let prev = -Infinity;
      for (const c of data) {
        const ts = Date.parse(c.timestamp);
        if (ts <= prev) ordered = false;
        if (ts === prev) dup = true;
        if (ts % step !== 0) aligned = false;
        prev = ts;
        const [o, h, l, cl] = [c.open, c.high, c.low, c.close].map(Number);
        for (const f of ["open", "high", "low", "close", "volume_base", "volume_quote_usd", "vwap"])
          if (!(typeof c[f] === "string" && NUM_RE.test(c[f]))) numeric = false;
        if (!(l <= Math.min(o, cl) && Math.max(o, cl) <= h)) ohlc = false;
      }
      if (data.length) {
        record(tag, a.id, "timestamps strictly increasing", ordered && !dup);
        record(tag, a.id, `timestamps aligned to ${gran}`, aligned);
        record(tag, a.id, "low <= open,close <= high on every bucket", ohlc);
        record(tag, a.id, "all OHLCV values are decimal strings", numeric);
        // Both window ends are inclusive (measured 2026-08-19: a 5-day
        // start/end range returns 6 buckets). Undocumented in §4 — flagged as
        // a docs gap by the task, but the check follows the implementation.
        const inWindow = data.every(
          (c) => Date.parse(c.timestamp) >= start.getTime() && Date.parse(c.timestamp) <= end.getTime()
        );
        record(tag, a.id, "all buckets inside the requested window (inclusive ends)", inWindow);
      }
    } else if (r.json) {
      validateAgainstSpec(tag, a.id, "/v1/assets/{asset_identifier}/ohlcv", "get", r.status, r.json);
    }
  }

  // GET /v1/oracles/{id}
  {
    const r = await api(`/v1/oracles/${enc}`);
    record("oracles", a.id, "returns 200 or documented 404", [200, 404].includes(r.status), `status ${r.status}`);
    if (r.json)
      validateAgainstSpec("oracles", a.id, "/v1/oracles/{asset_identifier}", "get", r.status, r.json);
    if (r.status === 200) allPropsPresent("oracles", a.id, "OraclesResponse", r.json);
  }
}

// Group 5: GET /v1/assets pagination — walk to exhaustion.
{
  console.log(`## pagination walk`);
  const seen = new Map(); // identity key -> count
  let cursor = null, pages = 0, lastHasMore = null, schemaOk = true;
  do {
    const q = cursor ? `limit=200&cursor=${encodeURIComponent(cursor)}` : "limit=200";
    const r = await api(`/v1/assets?${q}`);
    if (r.status !== 200) {
      record("list", "-", `page ${pages + 1} returns 200`, false, `status ${r.status}`);
      break;
    }
    if (!ajv.validate({ $ref: responseSchemaRef("/v1/assets", "get", 200) }, r.json)) schemaOk = false;
    for (const item of r.json.data) {
      const key = `${item.asset_code}|${item.issuer_address}|${item.contract_address}`;
      seen.set(key, (seen.get(key) || 0) + 1);
    }
    record("list", "-", `page ${pages + 1}: has_more consistent with cursor`,
      r.json.has_more === (r.json.cursor != null),
      `has_more=${r.json.has_more} cursor=${r.json.cursor == null ? "null" : "set"}`);
    cursor = r.json.cursor;
    lastHasMore = r.json.has_more;
    pages++;
  } while (cursor && pages < 100);
  record("list", "-", "every page validates against the schema", schemaOk);
  record("list", "-", "walk terminates (has_more=false on last page)", lastHasMore === false && pages < 100, `${pages} pages`);
  const dups = [...seen.entries()].filter(([, n]) => n > 1);
  record("list", "-", "no asset appears twice across the walk", dups.length === 0,
    dups.slice(0, 5).map(([k, n]) => `${k}×${n}`).join(", "));
  record("list", "-", "walk yields a plausible asset count (>=200)", seen.size >= 200, `${seen.size} assets`);
  console.log(`  ${pages} pages, ${seen.size} distinct assets`);
}

// Group 6: POST /v1/prices/batch vs the per-asset singles.
{
  console.log(`## batch vs single`);
  const ids = ASSETS.map((a) => a.id);
  const r = await api(`/v1/prices/batch`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ assets: ids }),
  });
  record("batch", "-", "returns 200", r.status === 200, `status ${r.status}`);
  if (r.status === 200 && validateAgainstSpec("batch", "-", "/v1/prices/batch", "post", 200, r.json)) {
    const returned = new Map(r.json.prices.map((p) => [p.asset, p]));
    const partition = [...returned.keys(), ...r.json.not_found].sort();
    record("batch", "-", "prices + not_found partition the request exactly",
      JSON.stringify(partition) === JSON.stringify([...ids].sort()),
      `got ${returned.size} prices + ${r.json.not_found.length} not_found of ${ids.length}`);
    for (const [id, single] of singlePrices) {
      const b = returned.get(id);
      if (!b) {
        record("batch", id, "asset with a single price is present in batch", false);
        continue;
      }
      if (b.updated_at === single.updated_at) {
        const same = ["price_usd", "price_xlm", "vwap_24h", "volume_24h_usd"].every((f) => b[f] === single[f]);
        record("batch", id, "batch equals single at the same timestamp", same);
      } else {
        // Gateway caches /price for 10 s while batch is uncached — re-fetch the
        // single once; by now the cached entry has long expired.
        const r2 = await api(`/v1/assets/${encodeURIComponent(id)}/price`);
        const s2 = r2.json;
        if (r2.status === 200 && s2.updated_at === b.updated_at) {
          const same = ["price_usd", "price_xlm", "vwap_24h", "volume_24h_usd"].every((f) => b[f] === s2[f]);
          record("batch", id, "batch equals re-fetched single at the same timestamp", same);
        } else {
          skip("batch", id, "batch/single timestamps never aligned", `batch=${b.updated_at} single=${s2?.updated_at}`);
        }
      }
    }
  }
}

// Group 7: GET /v1/backfill/status.
{
  console.log(`## backfill status`);
  const r = await api(`/v1/backfill/status`);
  record("backfill", "-", "returns 200", r.status === 200, `status ${r.status}`);
  if (r.status === 200 && validateAgainstSpec("backfill", "-", "/v1/backfill/status", "get", 200, r.json)) {
    allPropsPresent("backfill", "-", "BackfillStatus", r.json);
  }
}

// ---------- summary ----------
const counts = { pass: 0, fail: 0, skip: 0 };
for (const c of checks) counts[c.status]++;
const report = {
  task: "0120",
  run_at: startedAt.toISOString(),
  duration_s: Math.round((Date.now() - startedAt.getTime()) / 1000),
  base_url: BASE_URL,
  spec_version: spec?.info?.version,
  asset_list: { file: "tools/scripts/conformance-assets.json", derived_at: assetsFile.derived_at, count: ASSETS.length },
  summary: counts,
  checks,
};
const out = `conformance-0120-report-${startedAt.toISOString().replace(/[:.]/g, "").slice(0, 15)}.json`;
writeFileSync(out, JSON.stringify(report, null, 2));

console.log(`\n# Summary: ${counts.pass} pass, ${counts.fail} fail, ${counts.skip} skip`);
const failGroups = {};
for (const c of checks) if (c.status === "fail") failGroups[c.group] = (failGroups[c.group] || 0) + 1;
for (const [g, n] of Object.entries(failGroups)) console.log(`  ${g}: ${n} failing`);
console.log(`report: ${out}`);
process.exit(counts.fail ? 1 : 0);
