// The sign-up notifier (task 0355): decodes a subscription batch and words
// the Slack message. The AWS calls are not run here.
//
// Run: npx nx test @rumblefish/stellar-prices-api-aws-cdk

import assert from 'node:assert/strict';
import { test } from 'node:test';
import { gzipSync } from 'node:zlib';

import {
  keyIds,
  notification,
} from '../../infra/lambda/signup-notifier/index.mjs';

// A production line's shape (2026-11-04), `created` flipped to true.
const line = (keyId) =>
  JSON.stringify({
    timestamp: '2026-11-04T13:15:02.516748Z',
    level: 'INFO',
    fields: {
      message: 'portal issued an API key',
      key_id: keyId,
      created: true,
    },
    span: { name: 'Lambda runtime invoke' },
  });

const batch = (...messages) => ({
  awslogs: {
    data: gzipSync(
      JSON.stringify({
        logEvents: messages.map((message, i) => ({ id: String(i), message })),
      }),
    ).toString('base64'),
  },
});

test('keyIds reads every key id in the batch', () => {
  assert.deepEqual(keyIds(batch(line('abc123'), line('def456'))), [
    'abc123',
    'def456',
  ]);
});

const dashboard = 'https://example.test/dashboard';

test('a first portal key: profile link, ID, plan, change-plan', () => {
  const { source, content } = notification(
    {
      name: 'discord-1534853384740540537-key',
      keyId: 'xtzde0tusb',
      plans: ['pricing-api-free-production'],
      issuedBy: 'task-0187',
      sameName: 1,
    },
    dashboard,
  );
  assert.equal(source, 'custom');
  assert.equal(content.title, ':tada: New Prices API key');
  const [who, change, key] = content.description.split('\n');
  assert.equal(
    who,
    '<https://discord.com/users/1534853384740540537|Discord profile> · ID `1534853384740540537` · free plan · via portal · first key for this user',
  );
  assert.match(change, /^Change the plan: `\/change-plan 1534853384740540537`/);
  assert.equal(
    key,
    `Key \`xtzde0tusb\` · <${dashboard}|Sign-ups per day on the dashboard>`,
  );
});

test('a re-issued hub key keeps its paid plan', () => {
  const { content } = notification(
    {
      name: 'discord-1-key',
      keyId: 'k',
      plans: ['pricing-api-pro-production'],
      issuedBy: 'hub-contract',
      sameName: 3,
    },
    dashboard,
  );
  assert.match(
    content.description,
    /· pro plan · via API hub · re-issued \(3 keys under this name\)/,
  );
});

test('a key not named after a Discord ID gets no link and no change-plan', () => {
  const { content } = notification(
    {
      name: 'loadtest',
      keyId: 'k',
      plans: [],
      issuedBy: undefined,
      sameName: 1,
    },
    dashboard,
  );
  assert.doesNotMatch(content.description, /discord\.com|change-plan/);
  assert.match(
    content.description,
    /^Key name `loadtest` · no plan · via unknown/,
  );
});
