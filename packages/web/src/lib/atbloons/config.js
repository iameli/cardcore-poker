/**
 * atbloons paid-hand configuration for the Cardcore client.
 *
 * The paid flow is OFF by default. A Cardcore deployment turns it on by
 * supplying a wallet public origin. When no configuration is present, the game
 * runs its normal unpaid hands with no atbloons code path.
 *
 * The wallet is the atbloons node's managed wallet, so the wallet origin is
 * also the node origin. A deployment therefore needs to supply only the wallet
 * URL: the client discovers the exact network tuple from the node's public
 * `GET /v1/network` descriptor (`discoverAtbloonsConfig`). A deployment may
 * still pin the tuple by hand when it does not want a discovery request.
 *
 * Precedence, highest first:
 *   1. A per-browser override in `localStorage` (`atbloons.config`).
 *   2. Build-time Vite env (`VITE_ATBLOONS_*`).
 *
 * The network tuple must match the operator's atbloons node exactly. atbloons
 * is a resettable testnet, not a blockchain or mainnet.
 */

const STORAGE_KEY = "atbloons.config";
const DISCOVERED_KEY = "atbloons.config.discovered";

/** A genesis hash is a 64-character lowercase hex SHA-256 digest. */
function isGenesisHash(value) {
  return typeof value === "string" && /^[0-9a-f]{64}$/.test(value);
}

/** Trim a trailing slash so `${base}/v1/network` never doubles the separator. */
function trimTrailingSlash(url) {
  return typeof url === "string" ? url.replace(/\/+$/, "") : url;
}

function readEnv() {
  // `import.meta.env` exists under Vite. Guard for node test contexts.
  const env = typeof import.meta !== "undefined" && import.meta.env ? import.meta.env : {};
  const walletBaseUrl = env.VITE_ATBLOONS_WALLET_URL;
  if (!walletBaseUrl) return null;
  const networkId = env.VITE_ATBLOONS_NETWORK_ID;
  const protocolVersion = env.VITE_ATBLOONS_PROTOCOL_VERSION;
  const genesisHash = env.VITE_ATBLOONS_GENESIS_HASH;
  // A hand-pinned tuple keeps the wallet from making a discovery request.
  if (networkId && genesisHash) {
    return {
      walletBaseUrl,
      scope: { networkId, protocolVersion: protocolVersion || "v3", genesisHash },
    };
  }
  // Otherwise the wallet URL alone enables discovery from the node descriptor.
  return { walletBaseUrl };
}

function readOverride(storage) {
  try {
    const raw = storage.getItem(STORAGE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw);
    if (!parsed.walletBaseUrl) return null;
    // A stored override may carry only a wallet URL (discovery still pending)
    // or a full pinned tuple.
    return parsed;
  } catch {
    return null;
  }
}

function storageOrNull() {
  try {
    return typeof globalThis !== "undefined" && globalThis.localStorage
      ? globalThis.localStorage
      : null;
  } catch {
    return null;
  }
}

/** The base configuration, before any node discovery. May lack a scope. */
function readBaseConfig(storage) {
  const override = storage ? readOverride(storage) : null;
  return override || readEnv();
}

/** A cached full config from a previous discovery, if it matches the base URL. */
function readDiscoveredCache(storage, walletBaseUrl) {
  if (!storage) return null;
  try {
    const raw = storage.getItem(DISCOVERED_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw);
    if (parsed.walletBaseUrl !== walletBaseUrl) return null;
    if (!isFullConfig(parsed)) return null;
    return parsed;
  } catch {
    return null;
  }
}

/** A config is complete when it names a wallet URL and a valid network tuple. */
function isFullConfig(config) {
  return !!(
    config &&
    config.walletBaseUrl &&
    config.scope &&
    config.scope.networkId &&
    config.scope.protocolVersion &&
    isGenesisHash(config.scope.genesisHash)
  );
}

/**
 * Resolve the effective atbloons configuration without a network request.
 *
 * Returns a full config (with a network tuple) when the tuple was pinned by
 * hand or previously discovered and cached. Returns a partial config (only a
 * `walletBaseUrl`) when a wallet is configured but the tuple must still be
 * discovered from the node. Returns `null` when the paid flow is not
 * configured — run unpaid, show no atbloons UI.
 * @returns {{walletBaseUrl:string, scope?:{networkId:string,protocolVersion:string,genesisHash:string}}|null}
 */
export function resolveAtbloonsConfig({ storage: storageArg } = {}) {
  const storage = storageArg !== undefined ? storageArg : storageOrNull();
  const base = readBaseConfig(storage);
  if (!base) return null;
  if (isFullConfig(base)) return base;
  const cached = readDiscoveredCache(storage, base.walletBaseUrl);
  return cached || base;
}

/**
 * Resolve a full atbloons configuration, discovering the network tuple from the
 * node's managed-wallet descriptor when only a wallet URL is configured.
 *
 * The wallet is the atbloons node's managed wallet, so the wallet origin also
 * serves `GET /v1/network`. A successful discovery is cached per browser so
 * later loads resolve synchronously. Any failure returns `null`: the paid flow
 * stays unavailable and unpaid play is unchanged.
 * @param {object} [opts]
 * @param {typeof fetch} [opts.fetch] - fetch implementation, injectable for tests
 * @returns {Promise<{walletBaseUrl:string, scope:{networkId:string,protocolVersion:string,genesisHash:string}}|null>}
 */
export async function discoverAtbloonsConfig({ fetch: fetchImpl, storage: storageArg } = {}) {
  const storage = storageArg !== undefined ? storageArg : storageOrNull();
  const base = readBaseConfig(storage);
  if (!base) return null;
  if (isFullConfig(base)) return base;

  const cached = readDiscoveredCache(storage, base.walletBaseUrl);
  if (cached) return cached;

  const doFetch = fetchImpl || (typeof globalThis !== "undefined" ? globalThis.fetch : undefined);
  if (typeof doFetch !== "function") return null;

  const nodeUrl = trimTrailingSlash(base.walletBaseUrl);
  let descriptor;
  try {
    const res = await doFetch(`${nodeUrl}/v1/network`, {
      headers: { accept: "application/json" },
    });
    if (!res || !res.ok) return null;
    descriptor = await res.json();
  } catch {
    return null;
  }

  const scope = descriptor && descriptor.scope;
  if (
    !scope ||
    typeof scope.networkId !== "string" ||
    typeof scope.protocolVersion !== "string" ||
    !isGenesisHash(scope.genesisHash)
  ) {
    return null;
  }

  const resolved = {
    walletBaseUrl: base.walletBaseUrl,
    scope: {
      networkId: scope.networkId,
      protocolVersion: scope.protocolVersion,
      genesisHash: scope.genesisHash,
    },
  };
  if (storage) {
    try {
      storage.setItem(DISCOVERED_KEY, JSON.stringify(resolved));
    } catch {
      // A read-only or full store simply means no cache; discovery still works.
    }
  }
  return resolved;
}

/** Save a per-browser wallet override. Pass `null` to clear it. */
export function setAtbloonsConfigOverride(config) {
  const storage = storageOrNull();
  if (!storage) return;
  if (config == null) {
    storage.removeItem(STORAGE_KEY);
    storage.removeItem(DISCOVERED_KEY);
    return;
  }
  storage.setItem(STORAGE_KEY, JSON.stringify(config));
  // A new override invalidates any tuple discovered for the previous wallet.
  storage.removeItem(DISCOVERED_KEY);
}

/**
 * The atbloons paid flow is available when a wallet is configured, even before
 * the network tuple is discovered. The panel renders and completes discovery.
 */
export function isAtbloonsEnabled() {
  return resolveAtbloonsConfig() != null;
}
