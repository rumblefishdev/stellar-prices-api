import { act, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';

import { BASE_PATH } from './base-path';

/**
 * The only test that exercises `main.tsx`, and the only one that can fail if
 * `basename` is dropped from the router.
 *
 * `app.spec.tsx` mounts `App` inside a `MemoryRouter` the test itself
 * configures, so it proves the routes work under a prefix without ever proving
 * that the real entry point passes one. This mounts the actual entry point at
 * the actual URL the app is served from: with `basename` in place the `/` route
 * matches and the heading renders; without it `BrowserRouter` reads
 * `/api/` as a path it has no route for and renders an empty tree — which
 * is precisely the production failure that is invisible everywhere else.
 */
beforeEach(() => {
  vi.resetModules();
  vi.stubGlobal(
    'fetch',
    vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ enabled: false }),
    }),
  );
  // `main.tsx` renders into this and throws on a null element by design.
  const root = document.createElement('div');
  root.id = 'root';
  document.body.appendChild(root);
  // BrowserRouter reads `window.location`, so the URL has to be the one the
  // distribution actually serves before the module is imported.
  window.history.pushState({}, '', BASE_PATH);
});

afterEach(() => {
  vi.unstubAllGlobals();
  document.body.innerHTML = '';
  window.history.pushState({}, '', '/');
});

it('mounts the app at the prefix the bundle is served from', async () => {
  // The import IS the render — `main.tsx` calls `createRoot(...).render(...)` at
  // module scope — so it has to happen inside `act` or React warns that a state
  // update escaped it.
  await act(async () => {
    await import('./main');
  });

  await waitFor(() =>
    expect(screen.getByRole('heading', { level: 1 })).toBeTruthy(),
  );
  expect(await screen.findByText(/not yet available/i)).toBeTruthy();
});
