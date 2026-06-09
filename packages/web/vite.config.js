import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import metadataTemplate from "./oauth-client-metadata.template.json" with { type: "json" };

const SERVER_HOST = "127.0.0.1";
const SERVER_PORT = process.env.VITE_PORT || 5173;
const PDS_PORT = process.env.PDS_PORT || 2583;

// Apply OAUTH_HOST override (if set) to every URL field in the metadata. The
// resulting object is what gets emitted as dist/oauth-client-metadata.json
// AND what feeds the bundle's VITE_OAUTH_* env vars, so the served metadata
// always matches what the bundle thinks its client_id is.
function buildMetadata() {
  const host = process.env.OAUTH_HOST;
  if (!host) return metadataTemplate;
  const swap = (s) => s.replace("cardco.re", host);
  return {
    ...metadataTemplate,
    client_id: swap(metadataTemplate.client_id),
    client_uri: swap(metadataTemplate.client_uri),
    redirect_uris: metadataTemplate.redirect_uris.map(swap),
  };
}

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
        const metadata = buildMetadata();
        if (command === "build") {
          // Production: client_id is the URL where the metadata JSON is
          // hosted. We emit the (possibly OAUTH_HOST-rewritten) metadata as
          // a build asset below, so this URL points at a real file.
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

        // Filtered firehose. In prod we default to firehose.channel, which
        // accepts wantedDids (gameplay: only our peers' commits) and
        // wantedCollections (lobby discovery: only re.cardco.poker.table
        // commits) query params — so one socket never drowns in unrelated
        // network traffic. In dev we leave it empty and fall back to the
        // local PDS, which already has just our demo accounts.
        const defaultFirehose = command === "build" ? "wss://firehose.channel" : "";
        process.env.VITE_FIREHOSE_URL = process.env.FIREHOSE_URL ?? defaultFirehose;
      },
      configureServer(server) {
        // Serve the (possibly rewritten) metadata in dev too. Dev OAuth uses
        // the localhost trick so the PDS doesn't fetch this — but anything
        // proxying through an HTTPS hostname will, and it has to match.
        server.middlewares.use("/oauth-client-metadata.json", (_req, res) => {
          res.setHeader("content-type", "application/json");
          res.end(JSON.stringify(buildMetadata(), null, 2) + "\n");
        });
      },
      generateBundle() {
        this.emitFile({
          type: "asset",
          fileName: "oauth-client-metadata.json",
          source: JSON.stringify(buildMetadata(), null, 2) + "\n",
        });
      },
    },
    svelte(),
  ],
});
