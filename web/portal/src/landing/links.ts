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
 * All same-origin and root-relative — and all under `/api/`, because on the
 * shared host the root belongs to the block explorer (task 0194): a link to
 * `/api-docs-json` there opens the explorer's page, not the document.
 */

import { API_ORIGIN } from '../api-origin';
import { ROUTER_BASENAME } from '../base-path';

/**
 * The public base of the data API, `/v1` included — what every snippet on the
 * landing and the quick start tells a reader to call.
 *
 * The API's own hostname (task 0194), not the execute-api origin the pages
 * carried until 2026-08-31. A literal rather than something derived from
 * `API_ORIGIN`: that value is empty on every build but the shared-host one,
 * and a snippet must name the same host wherever the page is served from.
 * `links.spec.ts` asserts it equals `apiBaseUrl` in
 * `infra/envs/production.json` — the value the OpenAPI `servers` block
 * advertises — so a reader following the quick start and a reader following
 * the spec are sent to the same place.
 */
export const PUBLIC_API_BASE_URL =
  'https://prices-api.sorobanscan.rumblefish.dev/v1';

/**
 * The OpenAPI document. Real, served, and the only docs artefact today.
 *
 * Same-origin, the alias under the portal prefix (`portal::OPENAPI_PATH` in
 * the API), not the root `/api-docs-json` that partners and the spec's
 * `servers` block use — the two are the same bytes from the same handler. On
 * the shared host (task 0194) the alias is a static-SPA path like every other
 * `/api/*` there, so the link goes to the root copy on the API's own hostname
 * instead — a document, opened by navigation, so no CORS is involved.
 */
export const OPENAPI_JSON = API_ORIGIN
  ? `${API_ORIGIN}/api-docs-json`
  : '/api/api-docs-json';

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
 * The official Stellar Discord — **Stellar Developers**, and no other server
 * (Adam, 2026-09-02). This snowflake is the guild the backend's membership
 * gate checks: it is the value of the SSM parameter
 * `/prices/production/discord-guild-id`, and the two are ONE FACT in two
 * places. `tools/scripts/verify-discord-guild.mjs` resolves the invite below
 * through Discord's API and asserts it lands here — the drift it guards
 * against (invite to one server, gate on another) was invisible until task
 * 0254 measured it, because nothing compared them.
 */
export const STELLAR_DISCORD_GUILD_ID = '897514728459468821';

/**
 * The registered vanity invite, resolving to `STELLAR_DISCORD_GUILD_ID`. The
 * other invites SDF publishes are personal and at least one is already dead
 * (task 0179) — do not swap this for one found in a thread.
 */
export const STELLAR_DISCORD_INVITE = 'https://discord.gg/stellardev';

/**
 * The server itself, for a visitor who is ALREADY in it — a member following
 * the invite is told by Discord they are already a member and sent nowhere.
 * Discord opens a guild by id at this path (a member's own client picks the
 * channel); an unscreened member lands on the rules prompt, which is the one
 * click the `pending_rules` refusal asks for.
 */
export const STELLAR_DISCORD_SERVER = `https://discord.com/channels/${STELLAR_DISCORD_GUILD_ID}`;

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
 * The login route, relative to the router's basename — so `/api/login`
 * once `ROUTER_BASENAME` is applied.
 *
 * ⚠️ This path only resolves on a hard refresh once the deployment answers it
 * with the portal's `index.html`. Until then S3 masks the missing key as
 * `403 AccessDenied` (the bucket grants `s3:GetObject`, not `s3:ListBucket`),
 * which is why `infra/.../portal-hosting-stack.ts` rewrites the app's known
 * routes to `/api/index.html` in `DirectoryIndexFn`. Add a route here
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
