import { TestNetwork } from "./dist/index.js";

// Fixed port so the Vite dev server can proxy /xrpc to it. Override with PDS_PORT.
const port = process.env.PDS_PORT ? Number(process.env.PDS_PORT) : 2583;

const network = await TestNetwork.create({ pds: { port } });

console.log(
  JSON.stringify({
    "pds-url": network.pds.url,
    "plc-url": network.plc.url,
  }),
);

const shutdown = async () => {
  await network.close();
  process.exit(0);
};
process.on("SIGINT", shutdown);
process.on("SIGTERM", shutdown);
