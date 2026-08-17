import { StrictMode } from 'react';
import * as ReactDOM from 'react-dom/client';
import { BrowserRouter } from 'react-router-dom';

import App from './app/app';
// The other half of `base` in `vite.config.mts`, which imports the same module:
// `base` makes the bundle's ASSETS resolve under `/api-tokens/`, this makes its
// ROUTES do the same. Without it the router reads `/api-tokens/` as a route it
// has never heard of and renders nothing, on the one URL the app is actually
// served from. See `base-path.ts` for why the two values differ by a slash.
import { ROUTER_BASENAME } from './base-path';

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
