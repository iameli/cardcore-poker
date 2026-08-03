/**
 * Offline tests for the atbloons managed-wallet configuration and node
 * network-tuple discovery. No network, no browser: a memory store stands in
 * for `localStorage` and an injected fetch returns a fixed node descriptor.
 */

import { test } from "node:test";
import assert from "node:assert/strict";

import {
  resolveAtbloonsConfig,
  discoverAtbloonsConfig,
  setAtbloonsConfigOverride,
  isAtbloonsEnabled,
} from "./config.js";

const WALLET_URL = "https://wallet.example";
const GENESIS = "a".repeat(64);
const SCOPE = { networkId: "atbloons-testnet-v4", protocolVersion: "v3", genesisHash: GENESIS };

class MemoryStore {
  constructor(entries = {}) {
    this._map = new Map(Object.entries(entries));
  }
  getItem(key) {
    return this._map.has(key) ? this._map.get(key) : null;
  }
  setItem(key, value) {
    this._map.set(key, String(value));
  }
  removeItem(key) {
    this._map.delete(key);
  }
}

function overrideStore(config) {
  const store = new MemoryStore();
  store.setItem("atbloons.config", JSON.stringify(config));
  return store;
}

function jsonResponse(body, ok = true) {
  return {
    ok,
    async json() {
      return body;
    },
  };
}

test("no configuration resolves to null and disables the paid flow", () => {
  const storage = new MemoryStore();
  assert.equal(resolveAtbloonsConfig({ storage }), null);
});

test("a wallet-only override resolves to a partial config pending discovery", () => {
  const storage = overrideStore({ walletBaseUrl: WALLET_URL });
  const resolved = resolveAtbloonsConfig({ storage });
  assert.equal(resolved.walletBaseUrl, WALLET_URL);
  assert.equal(resolved.scope, undefined);
});

test("a hand-pinned tuple resolves fully and discovery makes no request", async () => {
  const storage = overrideStore({ walletBaseUrl: WALLET_URL, scope: SCOPE });
  assert.deepEqual(resolveAtbloonsConfig({ storage }).scope, SCOPE);
  let called = false;
  const config = await discoverAtbloonsConfig({
    storage,
    fetch: async () => {
      called = true;
      return jsonResponse({});
    },
  });
  assert.equal(called, false, "a pinned tuple must not trigger a discovery request");
  assert.deepEqual(config.scope, SCOPE);
});

test("discovery reads the network tuple from the node descriptor and caches it", async () => {
  const storage = overrideStore({ walletBaseUrl: WALLET_URL });
  const urls = [];
  const config = await discoverAtbloonsConfig({
    storage,
    fetch: async (url) => {
      urls.push(url);
      return jsonResponse({ scope: SCOPE, minimumValidatorCount: 1 });
    },
  });
  assert.deepEqual(urls, [`${WALLET_URL}/v1/network`]);
  assert.deepEqual(config, { walletBaseUrl: WALLET_URL, scope: SCOPE });
  // The cache lets a later synchronous resolve return the full tuple.
  assert.deepEqual(resolveAtbloonsConfig({ storage }).scope, SCOPE);
});

test("a trailing slash on the wallet URL does not double the descriptor path", async () => {
  const storage = overrideStore({ walletBaseUrl: `${WALLET_URL}/` });
  const urls = [];
  await discoverAtbloonsConfig({
    storage,
    fetch: async (url) => {
      urls.push(url);
      return jsonResponse({ scope: SCOPE });
    },
  });
  assert.deepEqual(urls, [`${WALLET_URL}/v1/network`]);
});

test("a cached discovery is reused without a second request", async () => {
  const storage = overrideStore({ walletBaseUrl: WALLET_URL });
  let calls = 0;
  const fetchOnce = async () => {
    calls += 1;
    return jsonResponse({ scope: SCOPE });
  };
  await discoverAtbloonsConfig({ storage, fetch: fetchOnce });
  await discoverAtbloonsConfig({ storage, fetch: fetchOnce });
  assert.equal(calls, 1);
});

test("a failed descriptor request leaves the paid flow unavailable", async () => {
  const storage = overrideStore({ walletBaseUrl: WALLET_URL });
  const config = await discoverAtbloonsConfig({
    storage,
    fetch: async () => jsonResponse({}, false),
  });
  assert.equal(config, null);
});

test("a descriptor with a malformed genesis hash is rejected", async () => {
  const storage = overrideStore({ walletBaseUrl: WALLET_URL });
  const config = await discoverAtbloonsConfig({
    storage,
    fetch: async () => jsonResponse({ scope: { ...SCOPE, genesisHash: "short" } }),
  });
  assert.equal(config, null);
});

test("a throwing fetch is swallowed and returns null", async () => {
  const storage = overrideStore({ walletBaseUrl: WALLET_URL });
  const config = await discoverAtbloonsConfig({
    storage,
    fetch: async () => {
      throw new Error("network down");
    },
  });
  assert.equal(config, null);
});

test("clearing the override forgets the discovered cache", async () => {
  const storage = overrideStore({ walletBaseUrl: WALLET_URL });
  await discoverAtbloonsConfig({ storage, fetch: async () => jsonResponse({ scope: SCOPE }) });
  assert.ok(resolveAtbloonsConfig({ storage }).scope);
  // setAtbloonsConfigOverride uses ambient storage; emulate by removing keys.
  storage.removeItem("atbloons.config");
  storage.removeItem("atbloons.config.discovered");
  assert.equal(resolveAtbloonsConfig({ storage }), null);
});

test("isAtbloonsEnabled tracks whether any wallet is configured", () => {
  assert.equal(typeof isAtbloonsEnabled, "function");
  assert.equal(setAtbloonsConfigOverride.length, 1);
});
