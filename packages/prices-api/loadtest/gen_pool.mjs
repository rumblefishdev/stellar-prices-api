// Builds pool-wide.json (gitignored) next to this file by walking GET /v1/assets.
//
//   BASE_URL=… API_KEY=… node packages/prices-api/loadtest/gen_pool.mjs
//
// Run it in the SAME command chain as the wide regime, never earlier: /price only
// serves assets with a 1m candle in the last 24 h, and about a third of that long
// tail turns over daily. Measured 2026-09-18 — a pool generated 20 h before the
// run had lost 1,536 of its 4,039 assets to 404 (task 0293).
import { writeFileSync } from 'node:fs';

const { BASE_URL, API_KEY } = process.env;
if (!BASE_URL || !API_KEY) throw new Error('BASE_URL and API_KEY must be set');

// Native XLM is listed with BOTH contract_address and issuer_address empty, so a
// bare `code:issuer` yields `XLM:` — which the API answers 400, not 404, and
// setup() aborts the run on any non-404 (see the README).
const id = (a) => a.contract_address || (a.issuer_address ? `${a.asset_code}:${a.issuer_address}` : 'native');

const out = new Set();
let cursor = null;
do {
  const url = new URL(`${BASE_URL}/v1/assets`);
  url.searchParams.set('limit', '200');
  if (cursor) url.searchParams.set('cursor', cursor);
  const res = await fetch(url, { headers: { 'x-api-key': API_KEY } });
  if (!res.ok) throw new Error(`listing answered ${res.status} — check the key's usage plan`);
  const page = await res.json();
  page.data.forEach((a) => out.add(id(a)));
  cursor = page.cursor;
  await new Promise((r) => setTimeout(r, 1100)); // the free plan is 1 req/s; any key must survive this
} while (cursor);

const file = new URL('./pool-wide.json', import.meta.url);
writeFileSync(file, JSON.stringify([...out]));
console.log(`pool: ${out.size} asset(s) written to ${file.pathname} — record this count in the report`);
