/**
 * Every off-page destination the landing page names, in one place.
 *
 * Two of the three are placeholders **on purpose**, and centralising them is
 * what keeps that honest: the quickstart is task 0163's and the Swagger UI is
 * task 0195's, and neither has landed. Task 0193's brief says to link out to
 * both — "a key is only useful next to the thing that shows what to call" — so
 * the links exist and point at the one artefact that is actually served today,
 * the OpenAPI document. When 0195 lands its route, this file is the diff.
 *
 * All same-origin and root-relative. `/api-docs-json` is on task 0184's
 * distribution behind the same CloudFront behaviour as `/v1`, so it is not a
 * third-party request and does not widen the CSP.
 */

import { ROUTER_BASENAME } from '../base-path';

/** The OpenAPI document. Real, served, and the only docs artefact today. */
export const OPENAPI_JSON = '/api-docs-json';

/**
 * The API reference — today the raw OpenAPI document, because that is the
 * only reference that exists. Task 0195 mounts Swagger UI on it; when it
 * lands, this is the one constant to re-point.
 *
 * Was `API_REFERENCE`, aliased to the document above. Every "Swagger UI"
 * affordance then silently opened `/api-docs-json`: a name that promised a
 * page the portal does not have. Named for what it opens instead.
 */
export const API_REFERENCE = OPENAPI_JSON;

/**
 * The quick start — the in-app page built from the Figma `Quick start` frame
 * (`918:644`). A ROUTE, so it is a `RouterLink` target like the dashboard;
 * `QUICKSTART` is the same place as an absolute href for the plain `<a>`s on
 * the landing page. Task 0163's curl-by-curl walkthrough is what its
 * snippets are meant to say once the two are reconciled.
 */
export const QUICKSTART_ROUTE = '/quick-start';
export const QUICKSTART = `${ROUTER_BASENAME}${QUICKSTART_ROUTE}`;

/**
 * The registered vanity invite. The other invites SDF publishes are personal
 * and at least one is already dead (task 0179) — do not swap this for one
 * found in a thread.
 */
export const STELLAR_DISCORD_INVITE = 'https://discord.gg/stellardev';

/**
 * The in-page anchor every "Get API Key" control scrolls to — the login
 * section (Figma frame `778:2499`).
 *
 * Named `login` rather than `api-key` since that section arrived: the target is
 * where a visitor signs in, and the anchor is the one part of it that shows up
 * in a URL somebody may paste.
 */
export const LOGIN_ANCHOR = 'login';

/**
 * The login route, relative to the router's basename — so `/api-tokens/login`
 * once `ROUTER_BASENAME` is applied.
 *
 * ⚠️ This path only resolves on a hard refresh once the deployment answers it
 * with the portal's `index.html`. Until then S3 masks the missing key as
 * `403 AccessDenied` (the bucket grants `s3:GetObject`, not `s3:ListBucket`),
 * which is why `infra/.../portal-hosting-stack.ts` rewrites the app's known
 * routes to `/api-tokens/index.html` in `DirectoryIndexFn`. Add a route here
 * and you must add it there.
 */
export const LOGIN_ROUTE = '/login';

/** The dashboard route. Same deployment caveat as {@link LOGIN_ROUTE}. */
export const DASHBOARD_ROUTE = '/dashboard';

/**
 * The landing page as an absolute href, for the pages that are NOT it.
 *
 * The navbar's three links name sections of the landing page (`#features`,
 * `#get-started`, `#faq`); on the quick start those anchors do not exist, so
 * they are prefixed with this and become links back to the page that has
 * them. Same basename treatment as {@link QUICKSTART}.
 */
export const LANDING = `${ROUTER_BASENAME}/`;
