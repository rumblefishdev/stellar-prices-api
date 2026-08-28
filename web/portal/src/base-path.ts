/**
 * The prefix this app is served from — one definition, two consumers.
 *
 * Task 0184's CloudFront table routes `<app>/*` to that app's bundle, so this
 * bundle only ever answers under `/api/`. Getting there takes two
 * settings that must agree: Vite's `base` covers ASSETS, react-router's
 * `basename` covers ROUTES, and either one alone leaves the app broken on the
 * only URL it is served from.
 *
 * They differ by exactly one character and that difference is load-bearing, not
 * a copy-paste mistake — Vite treats a `base` without a trailing slash as a
 * prefix to concatenate and emits `/apiassets/…`, while react-router
 * strips a trailing `basename` and warns if given one. So `ROUTER_BASENAME` is
 * DERIVED rather than typed out a second time.
 *
 * `vite.config.mts` holds its own copy of `BASE_PATH`, because a config that
 * imports application source trips Vite's `configLoader: 'native'` warning on
 * every build. That copy is not left on trust: `base-path.spec.ts` loads the
 * real config and fails if the two stop matching.
 *
 * This is a BUILD-time concern. CloudFront cannot fix it after the fact, which
 * is why task 0185 put it in the first commit rather than discovering it on the
 * first deploy.
 */

/** Vite's `base`. Trailing slash required. */
export const BASE_PATH = '/api/';

/** react-router's `basename`. Same prefix, no trailing slash. */
export const ROUTER_BASENAME = BASE_PATH.replace(/\/$/, '');
