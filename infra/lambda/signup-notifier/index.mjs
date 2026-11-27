// Task 0355: one Slack message per new API key.
//
// A subscription filter on the api-handler log group sends this Lambda every
// `portal issued an API key` line with `created: true` (the same pattern as
// the PortalNewApiKeys metric filter). It reads the key from API Gateway and
// publishes a Chatbot custom notification to the sign-ups topic.
//
// Never the Discord username: the privacy policy keeps it to display and the
// session. The Discord ID is the key's identifier, and the profile link shows
// the username to whoever clicks, in Discord.
//
// Tested by tools/scripts/signup-notifier.test.mjs.

import { gunzipSync } from 'node:zlib';

const SKILL_URL =
  'https://github.com/rumblefishdev/stellar-prices-api/blob/develop/.claude/skills/change-plan/SKILL.md';

// The `IssuedBy` tag (packages/prices-api/src/portal/keys/gateway.rs).
const ISSUERS = { 'task-0187': 'portal', 'hub-contract': 'API hub' };

/**
 * The key ids in a CloudWatch Logs subscription batch. A CONTROL_MESSAGE
 * (Logs checking the destination) and a line that is not JSON carry none.
 */
export function keyIds(event) {
  const batch = JSON.parse(
    gunzipSync(Buffer.from(event.awslogs.data, 'base64')).toString('utf8'),
  );
  if (batch.messageType !== 'DATA_MESSAGE') return [];
  return batch.logEvents
    .map((e) => {
      try {
        return JSON.parse(e.message).fields?.key_id;
      } catch {
        return undefined;
      }
    })
    .filter(Boolean);
}

/** The Chatbot custom notification for one key. */
export function notification({ name, keyId, plans, issuedBy }, dashboardUrl) {
  const id = /^discord-(\d+)-key$/.exec(name)?.[1];
  const plan =
    plans.map((p) => /^pricing-api-([a-z]+)-/.exec(p)?.[1] ?? p).join(', ') ||
    'no';
  const who = id
    ? `<https://discord.com/users/${id}|Discord profile> · ID \`${id}\``
    : `Key name \`${name}\``;
  // No "first key" or "re-issue": by the time the line is logged, reconcile
  // has deleted the user's revoked records (portal/keys/mod.rs), so the key
  // always looks like the only one. A re-issue shows as a carried-over plan.
  const lines = [
    `${who} · ${plan} plan · via ${ISSUERS[issuedBy] ?? issuedBy ?? 'unknown'}`,
    ...(id
      ? [
          `Change the plan: \`/change-plan ${id}\` in Claude Code in the prices repo (<${SKILL_URL}|what the skill does>)`,
        ]
      : []),
    `Key \`${keyId}\` · <${dashboardUrl}|Sign-ups per day on the dashboard>`,
  ];
  return {
    version: '1.0',
    source: 'custom',
    content: {
      textType: 'client-markdown',
      title: ':tada: New Prices API key',
      description: lines.join('\n'),
    },
  };
}

export async function handler(event) {
  // Imported here so the tests load this file without the AWS SDK; the
  // Node.js Lambda runtime ships it.
  const api = await import('@aws-sdk/client-api-gateway');
  const { SNSClient, PublishCommand } = await import('@aws-sdk/client-sns');
  const gateway = new api.APIGatewayClient({});
  const sns = new SNSClient({});

  // One key at a time, each on its own: a key that fails (deleted already,
  // as our test keys are) costs only its own message. Throwing would make
  // Logs retry the whole batch and post the keys before it again.
  for (const keyId of keyIds(event)) {
    try {
      const key = await gateway.send(
        new api.GetApiKeyCommand({ apiKey: keyId }),
      );
      const plans = await gateway.send(new api.GetUsagePlansCommand({ keyId }));
      const message = notification(
        {
          name: key.name,
          keyId,
          plans: (plans.items ?? []).map((p) => p.name),
          issuedBy: key.tags?.IssuedBy,
        },
        process.env.DASHBOARD_URL,
      );
      await sns.send(
        new PublishCommand({
          TopicArn: process.env.TOPIC_ARN,
          Message: JSON.stringify(message),
        }),
      );
    } catch (error) {
      console.error(`sign-up message for key ${keyId} not sent:`, error);
    }
  }
}
