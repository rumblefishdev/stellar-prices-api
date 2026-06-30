// k6 load test for the §9 SLO:
//   100 req/s sustained for 5 minutes on GET /assets/{id}/price
//   → p95 latency < 200 ms, error rate < 0.1%
//
// Run against a deployed stage (authoritative) or the local server (approximate
// — no gateway cache / no Lambda cold start, so a lower bound on prod p95):
//
//   k6 run packages/prices-api/loadtest/price_load.js \
//     -e BASE_URL=https://<api>/<stage> -e API_KEY=<key> -e ASSET=native
//
// Knobs (env): RATE (req/s, default 100), DURATION (default 5m), ASSET,
// API_KEY, VUS, MAX_VUS. k6 exits non-zero if a threshold is breached.

import http from 'k6/http';
import { check } from 'k6';

const BASE_URL = __ENV.BASE_URL || 'http://localhost:8080';
const ASSET = __ENV.ASSET || 'native';
const API_KEY = __ENV.API_KEY || '';
const RATE = Number(__ENV.RATE || 100);

export const options = {
  scenarios: {
    price: {
      executor: 'constant-arrival-rate',
      rate: RATE,
      timeUnit: '1s',
      duration: __ENV.DURATION || '5m',
      preAllocatedVUs: Number(__ENV.VUS || 50),
      maxVUs: Number(__ENV.MAX_VUS || 200),
    },
  },
  thresholds: {
    // p95 < 200 ms.
    http_req_duration: ['p(95)<200'],
    // error rate < 0.1%.
    http_req_failed: ['rate<0.001'],
  },
};

const PARAMS = {
  headers: API_KEY ? { 'x-api-key': API_KEY } : {},
  tags: { endpoint: 'price' },
};

export default function () {
  const res = http.get(`${BASE_URL}/v1/assets/${ASSET}/price`, PARAMS);
  check(res, {
    'status is 200': (r) => r.status === 200,
    'has price_usd': (r) => r.json('price_usd') !== undefined,
  });
}
