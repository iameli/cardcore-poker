import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import metadata from "./public/oauth-client-metadata.json" with { type: "json" };

const SERVER_HOST = "127.0.0.1";
const SERVER_PORT = process.env.VITE_PORT || 5173;
const PDS_PORT = process.env.PDS_PORT || 2583;

export default defineConfig({
  server: {
    host: SERVER_HOST,
    port: SERVER_PORT,
    allowedHosts: process.env.VITE_HOST ? [process.env.VITE_HOST] : [],
    proxy: {
      // AT Protocol XRPC endpoints (HTTP + WebSocket for firehose) → local PDS.
      // Production builds talk to the user's real PDS directly via OAuth, so
      // this proxy is only used in dev/demo mode.
      "/xrpc": {
        target: `http://localhost:${PDS_PORT}`,
        ws: true,
        changeOrigin: true,
      },
    },
  },
  plugins: [
    {
      name: "oauth-env",
      config(_conf, { command }) {
        if (command === "build") {
          // Production: client_id is the URL where metadata JSON is hosted
          process.env.VITE_OAUTH_CLIENT_ID = metadata.client_id;
          process.env.VITE_OAUTH_REDIRECT_URI = metadata.redirect_uris[0];
        } else {
          // Development: use the localhost query-string trick so the PDS
          // doesn't try to fetch metadata from unreachable 127.0.0.1
          const redirectUri =
            `http://${SERVER_HOST}:${SERVER_PORT}` + new URL(metadata.redirect_uris[0]).pathname;

          process.env.VITE_OAUTH_REDIRECT_URI = redirectUri;
          process.env.VITE_OAUTH_CLIENT_ID =
            `http://localhost` +
            `?redirect_uri=${encodeURIComponent(redirectUri)}` +
            `&scope=${encodeURIComponent(metadata.scope)}`;
        }
        process.env.VITE_OAUTH_SCOPE = metadata.scope;

        // Identity resolver. In dev we leave it empty so handle/DID lookups
        // hit the local PDS via Vite's /xrpc proxy (which knows .test demo
        // accounts). In prod we default to Slingshot, mary-ext's hosted
        // identity resolver. Override with SLINGSHOT_URL.
        const defaultSlingshot = command === "build" ? "https://slingshot.microcosm.blue" : "";
        process.env.VITE_SLINGSHOT_URL = process.env.SLINGSHOT_URL ?? defaultSlingshot;
      },
    },
    svelte(),
  ],
});
