/**
 * Every off-page destination the landing page names, in one place.
 *
 * Centralised so that a destination that does not exist yet cannot hide: the
 * quick start and the API reference were both placeholders aliased to the raw
 * OpenAPI document until their pages landed (task 0193 for the quick start's
 * page, task 0195 for Swagger UI), and every "Swagger UI" affordance on the
 * landing page went through this file rather than through a literal. Task
 * 0163's curl-by-curl walkthrough is still owed to the quick start's snippets.
 *
 * All root-relative and all under `/api/`, because on the shared host the root
 * belongs to the block explorer (task 0194): a link to `/api-docs-json` there
 * opens the explorer's page, not the document.
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
 * The OpenAPI document itself — the raw JSON, linked beside the rendered
 * reference for generators and other tooling.
 *
 * Same-origin, the alias under the portal prefix (`portal::OPENAPI_PATH` in
 * the API), not the root `/api-docs-json` that partners and the spec's
 * `servers` block use — the two are the same bytes from the same handler. On
 * the shared host (task 0194) the alias is a static-SPA path like every other
 * `/api/*` there, so this points at the root copy on the API's own hostname
 * instead.
 *
 * ⚠️ This is also what `docs/ApiReference.tsx` **fetches**, cross-origin, to
 * render the reference — which is why `GET /api-docs-json` answers
 * `Access-Control-Allow-Origin: *` (see `packages/prices-api/src/lib.rs`).
 * Opened by navigation it would need no such header; opened by `fetch` it
 * does, so the two must stay in step.
 */
export const OPENAPI_JSON = API_ORIGIN
  ? `${API_ORIGIN}/api-docs-json`
  : '/api/api-docs-json';

/**
 * The API reference route — the live OpenAPI document rendered in the
 * portal's own pieces, in Swagger UI's shape (task 0195,
 * `src/docs/ApiReference.tsx`). A ROUTE like the quick start, so it is a
 * `RouterLink` target inside the app; `API_REFERENCE` is the same place as an
 * absolute href for the plain `<a>`s on the landing page.
 *
 * On the shared host this URL works BECAUSE it is a route: the explorer's
 * `/api/*` behaviour rewrites every extensionless path to `/api/index.html`,
 * so `/api/docs` boots this bundle and the router renders the reference. A
 * static `docs/` folder in the bundle would have been reachable only as
 * `/api/docs/index.html`, which is why the reference is a page and not a
 * static folder.
 *
 * Until 2026-09-01 `API_REFERENCE` was an alias of {@link OPENAPI_JSON} —
 * the only reference that existed — and every "Swagger UI" affordance
 * silently opened the raw document.
 */
export const DOCS_ROUTE = '/docs';
export const API_REFERENCE = `${ROUTER_BASENAME}${DOCS_ROUTE}`;

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
 * A hard refresh on this path resolves because the host's `/api/*` behaviour
 * rewrites every extensionless path to `/api/index.html` — a rule, not an
 * allow-list, so adding a route here needs nothing on the hosting side. (Our
 * own distribution kept a per-route allow-list in `DirectoryIndexFn`; it was
 * retired by task 0195 once the page moved to the explorer's host.)
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
