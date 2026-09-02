import { describe, expect, it } from 'vitest';

import {
  apiKeyHeader,
  describe as describeSchema,
  displaySummary,
  scrubInternal,
  exampleOf,
  groupByTag,
  linkedComponent,
  operationId,
  primaryMedia,
  requiresKey,
  statusTone,
  typeLabel,
  type OpenApiDocument,
} from './openapi';
import { FIXTURE } from './openapi.fixture';

describe('typeLabel', () => {
  it('names components, primitives with formats, arrays and nullables', () => {
    const s = FIXTURE.components?.schemas ?? {};
    expect(
      typeLabel(FIXTURE, { $ref: '#/components/schemas/TypeFilter' }),
    ).toBe('TypeFilter');
    expect(typeLabel(FIXTURE, { type: 'integer', format: 'int32' })).toBe(
      'integer (int32)',
    );
    expect(typeLabel(FIXTURE, s.AssetListResponse.properties?.data)).toBe(
      'AssetListItem[]',
    );
    expect(typeLabel(FIXTURE, s.AssetListResponse.properties?.cursor)).toBe(
      'string | null',
    );
    // `oneOf: [null, $ref]` — utoipa's optional reference.
    expect(typeLabel(FIXTURE, s.AssetListItem.properties?.stream)).toBe(
      'Stream | null',
    );
    expect(typeLabel(FIXTURE, s.AssetListItem.properties?.updated_at)).toBe(
      'string (date-time)',
    );
    // No type at all: say so rather than guess.
    expect(typeLabel(FIXTURE, s.ErrorEnvelope.properties?.details)).toBe('any');
    expect(typeLabel(FIXTURE, undefined)).toBe('any');
  });

  it('links the component behind a reference, an optional reference and an array', () => {
    const s = FIXTURE.components?.schemas ?? {};
    expect(linkedComponent({ $ref: '#/components/schemas/TypeFilter' })).toBe(
      'TypeFilter',
    );
    expect(linkedComponent(s.AssetListItem.properties?.stream)).toBe('Stream');
    expect(linkedComponent(s.AssetListResponse.properties?.data)).toBe(
      'AssetListItem',
    );
    expect(linkedComponent({ type: 'string' })).toBeUndefined();
  });
});

describe('exampleOf', () => {
  it('builds a document from examples, enums, formats and references', () => {
    expect(
      exampleOf(FIXTURE, { $ref: '#/components/schemas/AssetListResponse' }),
    ).toEqual({
      data: [
        {
          asset_code: 'USDC',
          price_usd: 'string',
          stream: { ledger: 1 },
          updated_at: '2026-01-01T00:00:00Z',
        },
      ],
      cursor: 'string',
      has_more: true,
    });
    expect(
      exampleOf(FIXTURE, { $ref: '#/components/schemas/TypeFilter' }),
    ).toBe('classic');
    expect(
      exampleOf(FIXTURE, { $ref: '#/components/schemas/ErrorEnvelope' }),
    ).toEqual({
      code: 'invalid_id',
      message: 'string',
      details: null,
    });
  });

  it('terminates on a cycle through a union, not only through a property', () => {
    const doc: OpenApiDocument = {
      ...FIXTURE,
      components: {
        // A component that refers to itself sideways: no property to count
        // depth on, so only the hop bound stops this.
        schemas: { Loop: { oneOf: [{ $ref: '#/components/schemas/Loop' }] } },
      },
    };
    expect(exampleOf(doc, { $ref: '#/components/schemas/Loop' })).toBeNull();
  });

  it('terminates on a reference cycle', () => {
    const doc: OpenApiDocument = {
      ...FIXTURE,
      components: {
        schemas: {
          Node: {
            type: 'object',
            properties: { next: { $ref: '#/components/schemas/Node' } },
          },
        },
      },
    };
    const value = exampleOf(doc, { $ref: '#/components/schemas/Node' });
    expect(JSON.stringify(value).length).toBeLessThan(200);
  });
});

describe('groupByTag', () => {
  it('keeps the document order and appends tags it uses but never declares', () => {
    const groups = groupByTag(FIXTURE);
    expect(groups.map((g) => g.name)).toEqual(['ops', 'assets', 'prices']);
    expect(groups[0].description).toBe('Operational endpoints');
    expect(groups[2].operations.map((o) => `${o.method} ${o.path}`)).toEqual([
      'post /v1/prices/batch',
    ]);
    expect(groups[1].operations[0].id).toBe(operationId('get', '/v1/assets'));
  });
});

describe('requiresKey', () => {
  it('reads the operation override before the document default', () => {
    expect(requiresKey(FIXTURE, FIXTURE.paths['/health'].get)).toBe(false);
    expect(requiresKey(FIXTURE, FIXTURE.paths['/v1/assets'].get)).toBe(true);
    expect(
      requiresKey(
        { ...FIXTURE, security: undefined },
        FIXTURE.paths['/v1/assets'].get,
      ),
    ).toBe(false);
  });
});

describe('describe', () => {
  it('reads the description off an optional reference, then off the component', () => {
    const s = FIXTURE.components?.schemas ?? {};
    // `oneOf: [null, { $ref, description }]` — the field's own words sit on
    // the variant.
    expect(
      describeSchema(FIXTURE, {
        oneOf: [
          { type: 'null' },
          { $ref: '#/components/schemas/Stream', description: 'The stream.' },
        ],
      }),
    ).toBe('The stream.');
    // A bare reference falls back to what the component says about itself.
    expect(
      describeSchema(FIXTURE, { $ref: '#/components/schemas/TypeFilter' }),
    ).toBe('Which assets.');
    expect(
      describeSchema(FIXTURE, s.AssetListItem.properties?.asset_code),
    ).toMatch(/^The code\./);
    expect(describeSchema(FIXTURE, { type: 'string' })).toBeUndefined();
  });
});

describe('displaySummary', () => {
  it('drops the route the row already shows, and capitalises the rest', () => {
    // The remainder was written as a sentence tail, so it needs the capital
    // the route used to supply — `"this OpenAPI document."` on its own reads
    // as a fragment.
    expect(displaySummary('`GET /assets` — paginated list.')).toBe(
      'Paginated list.',
    );
    expect(displaySummary('`POST /prices/batch` — current prices.')).toBe(
      'Current prices.',
    );
    expect(displaySummary('List assets.')).toBe('List assets.');
    expect(displaySummary('`GET /health`')).toBe('`GET /health`');
    expect(displaySummary(undefined)).toBeUndefined();
  });
});

describe('scrubInternal', () => {
  it('removes task, ADR, PR and overview-section references, tidily', () => {
    expect(scrubInternal('24h percentage change (task 0072).')).toBe(
      '24h percentage change.',
    );
    expect(
      scrubInternal('Latest **priced** USD close (task 0135): a candle'),
    ).toBe('Latest **priced** USD close: a candle');
    expect(
      scrubInternal(
        'within the 0135 carry bound (general-overview §3.3 / §4.2). `{}` means',
      ),
    ).toBe('within the carry bound. `{}` means');
    expect(scrubInternal('A source is absent when the §5.5 threshold')).toBe(
      'A source is absent when the threshold',
    );
    expect(
      scrubInternal(
        "(mixed case like `uSd` is a 400 — task 0119's exact-token policy)",
      ),
    ).toBe('');
    expect(
      scrubInternal('decided in ADR 0008 and PR #268, see overview §4.2.'),
    ).toBe('decided in and, see.');
    // Nothing to scrub: byte for byte.
    expect(
      scrubInternal(
        'Opaque cursor for the next page (`null` on the last page).',
      ),
    ).toBe('Opaque cursor for the next page (`null` on the last page).');
    // A year is not a task id.
    expect(scrubInternal('since 2026-08-31, at 12:00')).toBe(
      'since 2026-08-31, at 12:00',
    );
    // A decimal is not a task id: the earlier bare `0\d{3}` rule turned this
    // into `a threshold of 0.` and would have mangled published text.
    expect(scrubInternal('a threshold of 0.0500 USD')).toBe(
      'a threshold of 0.0500 USD',
    );
    expect(scrubInternal('port 0800 stays')).toBe('port 0800 stays');
  });
});

describe('the rest', () => {
  it('names the key header, picks the JSON media type and tones a status', () => {
    expect(apiKeyHeader(FIXTURE)).toBe('x-api-key');
    expect(
      primaryMedia({
        'text/plain': {},
        'application/json': { schema: { type: 'string' } },
      })?.mediaType,
    ).toBe('application/json');
    expect(primaryMedia(undefined)).toBeUndefined();
    expect(statusTone('200')).toBe('success');
    expect(statusTone('404')).toBe('warn');
    expect(statusTone('503')).toBe('error');
    expect(statusTone('default')).toBe('muted');
    expect(operationId('get', '/v1/assets/{asset_identifier}/price')).toBe(
      'op-get-v1-assets-asset-identifier-price',
    );
  });
});
