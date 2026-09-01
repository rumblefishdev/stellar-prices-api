/// <reference types="vite/client" />

interface ImportMetaEnv {
  /**
   * The API's own hostname, baked in for the shared-host build only — see
   * `src/api-origin.ts`. Absent (relative URLs) everywhere else.
   */
  readonly VITE_PORTAL_API_ORIGIN?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
