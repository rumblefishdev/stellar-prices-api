// Tests for getAssignee in generate-lore-board.mjs (task 0327) and its use in
// board.html (task 0328).
//
// Run: node --test tools/scripts/generate-lore-board.test.mjs

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

import { getAssignee } from './generate-lore-board.mjs';

const e = (date, who) => ({ date, who });

const CASES = [
  {
    name: 'oldest-first history: newest is last',
    task: { history: [e('2026-09-01', 'okarcz'), e('2026-10-01', 'akot')] },
    want: 'akot',
  },
  {
    name: 'newest-first history: newest is first',
    task: { history: [e('2026-10-01', 'akot'), e('2026-09-01', 'okarcz')] },
    want: 'akot',
  },
  {
    name: 'out of order: the latest date wins',
    task: {
      history: [
        e('2026-09-01', 'okarcz'),
        e('2026-10-02', 'akot'),
        e('2026-09-15', 'stkrolikiewicz'),
      ],
    },
    want: 'akot',
  },
  {
    name: 'date tie, oldest-first: the later entry wins',
    task: {
      history: [
        e('2026-09-01', 'okarcz'),
        e('2026-10-01', 'stkrolikiewicz'),
        e('2026-10-01', 'akot'),
      ],
    },
    want: 'akot',
  },
  {
    name: 'date tie, newest-first: the earlier entry wins',
    task: {
      history: [
        e('2026-10-01', 'akot'),
        e('2026-10-01', 'stkrolikiewicz'),
        e('2026-09-01', 'okarcz'),
      ],
    },
    want: 'akot',
  },
  {
    name: 'assignee field overrides history',
    task: { assignee: 'akot', history: [e('2026-10-01', 'okarcz')] },
    want: 'akot',
  },
  {
    name: 'an agent entry is skipped',
    task: { history: [e('2026-10-02', 'claude'), e('2026-10-01', 'akot')] },
    want: 'akot',
  },
  {
    name: 'only agent entries: no assignee',
    task: { history: [e('2026-10-02', 'claude')] },
    want: null,
  },
  { name: 'empty history', task: { history: [] }, want: null },
  { name: 'no history', task: {}, want: null },
  {
    name: 'backlog gets no assignee',
    dir: 'backlog',
    task: { assignee: 'akot', history: [e('2026-10-01', 'akot')] },
    want: null,
  },
  {
    name: 'blocked gets no assignee',
    dir: 'blocked',
    task: { history: [e('2026-10-01', 'akot')] },
    want: null,
  },
  {
    name: 'archive keeps its assignee',
    dir: 'archive',
    task: { history: [e('2026-10-01', 'akot')] },
    want: 'akot',
  },
];

for (const { name, dir = 'active', task, want } of CASES) {
  test(name, () => {
    assert.equal(getAssignee({ _dir: dir, ...task }), want);
  });
}

// The page must show what board.json carries, not re-derive it (task 0328).
test('board.html reads the assignee from board.json', () => {
  const html = readFileSync(
    new URL('../../lore/board.html', import.meta.url),
    'utf-8',
  );
  const fn = html.match(/function getTaskAssignee\(t\) \{([\s\S]*?)\n\s*\}/);
  assert.ok(fn, 'getTaskAssignee not found in board.html');
  assert.equal(fn[1].trim(), 'return t.assignee || null;');
});
