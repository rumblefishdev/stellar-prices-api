#!/usr/bin/env node
/**
 * Assert the Discord invite the portal publishes lands on the guild the
 * portal's membership gate checks.
 *
 * WHY THIS EXISTS
 * ---------------
 * The official Stellar Discord is ONE server — Stellar Developers,
 * `897514728459468821` (Adam, 2026-09-02) — and the portal names it in two
 * places that nothing compared until task 0254:
 *
 *   - `web/portal/src/landing/links.ts` — `STELLAR_DISCORD_INVITE`, the button
 *     a refused visitor presses, and `STELLAR_DISCORD_GUILD_ID`, the id the
 *     `pending_rules` screen opens the server by;
 *   - SSM `/prices/<env>/discord-guild-id` — the guild the backend asks
 *     Discord about, seeded by hand (runbook §2a) and never by CDK.
 *
 * When they disagree the portal is a loop: the card says "join the server",
 * the button joins a DIFFERENT server, and the next attempt refuses again.
 * That was the state on 2026-09-02 (the gate on a test guild, the invite on
 * the real one) and it was invisible because the gate had never been walked.
 *
 * WHAT IT CHECKS
 * --------------
 *   1. `STELLAR_DISCORD_INVITE` resolves — through Discord's own invite API,
 *      unauthenticated — to `STELLAR_DISCORD_GUILD_ID`. Reading the link is
 *      not a check: a vanity code says nothing about where it lands.
 *   2. That guild has Membership Screening enabled
 *      (`MEMBER_VERIFICATION_GATE_ENABLED`). Reported, not enforced: it is
 *      SDF's setting on SDF's server, and if it goes the abuse barrier of
 *      ADR 0010 silently becomes "has a Discord account" — task 0170 owns the
 *      alert; this line is the cheapest place to notice.
 *   3. With `--ssm`, the live parameter equals the same id — the deployed
 *      gate and the published invite agree. Needs AWS credentials for the
 *      production account; skipped otherwise.
 *
 * Not in CI: it calls discord.com, and a Discord hiccup must not block an
 * unrelated merge. Run it when either value changes, and at deploy prep.
 *
 * Usage:
 *   npm run discord:verify-guild            # invite ↔ links.ts
 *   npm run discord:verify-guild -- --ssm   # …and ↔ SSM (production)
 */
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..', '..');
const LINKS = join(root, 'web', 'portal', 'src', 'landing', 'links.ts');
const SSM_PARAMETER = '/prices/production/discord-guild-id';
const SSM_REGION = 'eu-central-1';

const links = readFileSync(LINKS, 'utf8');
const constant = (name) => {
  const m = links.match(new RegExp(`export const ${name} = '([^']+)';`));
  if (!m) fail(`${LINKS} does not declare \`export const ${name} = '…'\``);
  return m[1];
};

const guildId = constant('STELLAR_DISCORD_GUILD_ID');
const invite = constant('STELLAR_DISCORD_INVITE');
if (!/^\d{17,20}$/.test(guildId)) {
  fail(`STELLAR_DISCORD_GUILD_ID is not a snowflake: ${guildId}`);
}
const code = invite.match(/^https:\/\/discord\.gg\/([A-Za-z0-9-]+)$/)?.[1];
if (!code) fail(`STELLAR_DISCORD_INVITE is not a discord.gg link: ${invite}`);

// 1. The invite, resolved by Discord rather than read.
const url = `https://discord.com/api/v10/invites/${code}?with_counts=true`;
const response = await fetch(url, { headers: { accept: 'application/json' } });
if (!response.ok) {
  fail(`GET ${url} → ${response.status}; cannot resolve the invite`);
}
const { guild, approximate_member_count: members } = await response.json();
if (!guild?.id)
  fail(`the invite resolved to no guild: ${JSON.stringify(guild)}`);

console.log(
  `invite   discord.gg/${code} → ${guild.id}  "${guild.name}"  (${members} members)`,
);
console.log(`links.ts STELLAR_DISCORD_GUILD_ID = ${guildId}`);
if (guild.id !== guildId) {
  fail(
    `the invite lands on ${guild.id} ("${guild.name}") but the portal opens and ` +
      `the gate checks ${guildId}. The official guild is Stellar Developers, ` +
      `897514728459468821 — fix whichever of the two is not it.`,
  );
}

// 2. The barrier the gate rents from SDF.
// Three states, not two. `features` missing altogether is a shape change or a
// partial guild object, NOT evidence that the gate is off — and reporting it
// as OFF is how an operator learns to ignore the one line that would tell
// them ADR 0010's abuse barrier is gone.
if (!Array.isArray(guild.features)) {
  console.log(
    'screening UNKNOWN — the invite carried no `features` array, so this run ' +
      'cannot say. Check the guild in Discord, and this script against the ' +
      'invite API if it keeps happening.',
  );
} else if (guild.features.includes('MEMBER_VERIFICATION_GATE_ENABLED')) {
  console.log('screening ON  (MEMBER_VERIFICATION_GATE_ENABLED)');
} else {
  console.log(
    'screening OFF — ⚠️  Membership Screening is disabled on this guild, so ' +
      'every joiner is `pending: false` immediately and the barrier is "has a ' +
      'Discord account". See task 0170.',
  );
}

// 3. The deployed gate, if asked.
if (process.argv.includes('--ssm')) {
  let value;
  try {
    value = execFileSync(
      'aws',
      [
        'ssm',
        'get-parameter',
        '--name',
        SSM_PARAMETER,
        '--region',
        SSM_REGION,
        '--query',
        'Parameter.Value',
        '--output',
        'text',
      ],
      { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] },
    ).trim();
  } catch (error) {
    fail(
      `aws ssm get-parameter ${SSM_PARAMETER} failed: ${error.stderr ?? error.message}`,
    );
  }
  console.log(`ssm      ${SSM_PARAMETER} = ${value}`);
  if (value !== guildId) {
    fail(
      `the deployed gate checks ${value} but the portal sends visitors to ${guildId}. ` +
        `Re-point with: aws ssm put-parameter --name ${SSM_PARAMETER} --value ${guildId} ` +
        `--type String --overwrite --region ${SSM_REGION}`,
    );
  }
}

console.log('ok — the invite, the portal and the gate name one guild');

function fail(message) {
  console.error(`verify-discord-guild: ${message}`);
  process.exit(1);
}
