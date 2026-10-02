// @vitest-environment node
/**
 * The quick start's example queries, run against the live API exactly as the
 * Copy button hands them out — task 0163's "every one of them was run against
 * production before publishing", and the commands task 0164 repeats with a
 * self-service key.
 *
 * Gated on `PRICES_API_KEY`, so CI skips it and a key never lives in a
 * workflow or a transcript (the incident task 0298 cleaned up after):
 *
 *   cd web/portal && PRICES_API_KEY=… npx vitest run QuickStart.live
 *
 * Each snippet's `text` — the string the Copy button writes — is parsed as
 * the curl it is (the quoted URL, `-X`, `-H`, `-d`), the placeholder is
 * swapped for the key, and the response is held to the fields the OpenAPI
 * document marks `required` on that route's 200 body. Nothing here is a
 * second copy of a request: what runs is what the page shows. On a failure
 * the report carries a status code and key names, never a body.
 *
 * Node rather than jsdom so `fetch` is the real one, and one call at a time —
 * four requests sit inside a free key's burst of five.
 */
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

import { describe, expect, it } from 'vitest';

import { EXAMPLES, EXAMPLE_PATHS, PLACEHOLDER_KEY } from './QuickStart';

const KEY = process.env.PRICES_API_KEY ?? '';

type Operation = {
  responses?: Record<
    string,
    { content?: Record<string, { schema?: { $ref?: string } }> }
  >;
};
type Spec = {
  paths: Record<string, Record<string, Operation>>;
  components: { schemas: Record<string, { required?: string[] }> };
};

/** The curl the Copy button writes, read back as a request. */
function parseCurl(text: string) {
  const url = text.match(/"(https:\/\/[^"]+)"/)?.[1];
  if (!url) throw new Error(`no quoted URL in: ${text}`);
  const method = text.match(/-X (\w+)/)?.[1] ?? 'GET';
  const headers = Object.fromEntries(
    [...text.matchAll(/-H "([^:"]+): ([^"]+)"/g)].map((m) => [
      m[1],
      m[2].replace(PLACEHOLDER_KEY, KEY),
    ]),
  );
  const body = text.match(/-d '([^']+)'/)?.[1];
  return { url, method, headers, body };
}

/**
 * The fields the spec requires on the route's 200 body — none when the route
 * publishes no schema, as `/health` does not.
 */
function requiredFields(spec: Spec, template: string, method: string) {
  const shape = (p: string) => p.replace(/\{[^}]+\}/g, '{}');
  const path = Object.keys(spec.paths).find(
    (p) => shape(p) === shape(template),
  );
  const ref =
    path &&
    spec.paths[path][method.toLowerCase()]?.responses?.['200']?.content?.[
      'application/json'
    ]?.schema?.$ref;
  const name = ref?.split('/').pop();
  return name ? (spec.components.schemas[name]?.required ?? []) : [];
}

describe.skipIf(!KEY)('quick-start examples against production', () => {
  const spec = JSON.parse(
    readFileSync(
      join(import.meta.dirname, '../../public/openapi.json'),
      'utf8',
    ),
  ) as Spec;

  it.each(Object.keys(EXAMPLES) as (keyof typeof EXAMPLES)[])(
    'the %s example answers 200 with every required field',
    async (key) => {
      const { url, method, headers, body } = parseCurl(EXAMPLES[key].text);
      const res = await fetch(url, { method, headers, body });
      const json: unknown = await res.json();
      expect(res.status).toBe(200);
      expect(Object.keys(json as object)).toEqual(
        expect.arrayContaining(
          requiredFields(spec, EXAMPLE_PATHS[key], method),
        ),
      );
    },
    15_000,
  );
});
