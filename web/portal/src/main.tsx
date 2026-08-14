import { StrictMode } from 'react';
import * as ReactDOM from 'react-dom/client';
import { BrowserRouter } from 'react-router-dom';

import App from './app/app';

/**
 * The other half of `base` in `vite.config.mts`.
 *
 * `base` makes the bundle's ASSETS resolve under `/api-tokens/`; this makes its
 * ROUTES do the same. Without it the router reads `/api-tokens/` as a route it
 * has never heard of and renders nothing, on the one URL the app is actually
 * served from.
 *
 * No trailing slash — react-router strips one and warns if it is given one,
 * which is the opposite of what Vite's `base` wants. The two constants
 * genuinely differ by that character; they are not a copy-paste mistake.
 */
const ROUTER_BASENAME = '/api-tokens';

const root = ReactDOM.createRoot(
  document.getElementById('root') as HTMLElement,
);

root.render(
  <StrictMode>
    <BrowserRouter basename={ROUTER_BASENAME}>
      <App />
    </BrowserRouter>
  </StrictMode>,
);
