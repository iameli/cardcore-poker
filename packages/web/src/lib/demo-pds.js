/**
 * Demo-mode signin against a local AT Protocol PDS.
 *
 * In development the Vite proxy forwards /xrpc to the local PDS booted by
 * `pnpm dev:pds`, so all PDS calls go to the same origin. In production
 * demo mode is disabled (no public PDS to write to without OAuth).
 *
 * Each browser context creates its own ephemeral account on the local PDS;
 * the session is stashed in localStorage and reused across reloads.
 */
import { CredentialManager, Client } from "@atcute/client";

const SESSION_KEY = "cardcore_demo_session";
// Vite proxies /xrpc to the local PDS, so we point the @atcute Client at our
// own origin and let the proxy forward.
const PDS_BASE = typeof window !== "undefined" ? window.location.origin : "http://127.0.0.1:5173";

function randomSlug() {
  return Math.random().toString(36).slice(2, 10);
}

/**
 * Create a fresh account on the local PDS and return a session.
 * The handle is `demo-{slug}.test` — the dev-env PDS is configured with `.test`
 * as a service handle domain.
 */
async function createDemoAccount() {
  const slug = randomSlug();
  const handle = `demo-${slug}.test`;
  const password = randomSlug();
  const email = `demo-${slug}@demo.invalid`;

  const res = await fetch("/xrpc/com.atproto.server.createAccount", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ handle, email, password }),
  });
  if (!res.ok) {
    const text = await res.text().catch(() => "");
    throw new Error(`createAccount failed (${res.status}): ${text}`);
  }
  const data = await res.json();
  // data: { did, handle, accessJwt, refreshJwt }
  return {
    did: data.did,
    handle: data.handle,
    accessJwt: data.accessJwt,
    refreshJwt: data.refreshJwt,
    pdsUri: window.location.origin, // we proxy via the Vite origin
    active: true,
    password, // kept so we can re-login if the JWT goes stale
  };
}

function loadStoredSession() {
  try {
    const raw = localStorage.getItem(SESSION_KEY);
    return raw ? JSON.parse(raw) : null;
  } catch {
    return null;
  }
}

function storeSession(session) {
  localStorage.setItem(SESSION_KEY, JSON.stringify(session));
}

function clearStoredSession() {
  localStorage.removeItem(SESSION_KEY);
}

/**
 * Build an @atcute/client Client backed by a CredentialManager that uses the
 * PDS-issued JWTs. The Client gives us `client.post("com.atproto.repo.createRecord", ...)`
 * etc. with auth applied automatically.
 */
function buildClient(sessionData) {
  const cm = new CredentialManager({
    service: PDS_BASE, // the Vite proxy forwards /xrpc/* to the PDS
    onSessionUpdate: (s) => storeSession({ ...sessionData, ...s }),
  });
  // Resume from our stored creds. CredentialManager handles refresh on 401.
  cm.session = {
    refreshJwt: sessionData.refreshJwt,
    accessJwt: sessionData.accessJwt,
    handle: sessionData.handle,
    did: sessionData.did,
    active: true,
  };
  return new Client({ handler: cm });
}

/**
 * Get-or-create a demo session for this browser. Returns an object with the
 * same shape the rest of the app expects from OAuth signins:
 *   { did, handle, name, client }
 */
export async function getOrCreateDemoSession() {
  let stored = loadStoredSession();
  if (!stored) {
    stored = await createDemoAccount();
    storeSession(stored);
  }
  // Validate the session with a cheap call; if it 401s, blow it away and retry.
  const client = buildClient(stored);
  try {
    const res = await client.get("com.atproto.server.getSession");
    if (!res.ok) throw new Error(`getSession ${res.status}`);
  } catch {
    clearStoredSession();
    stored = await createDemoAccount();
    storeSession(stored);
  }
  return {
    did: stored.did,
    handle: stored.handle,
    name: stored.handle,
    client: buildClient(stored),
    pdsUri: stored.pdsUri,
    isDemo: true,
  };
}

export function clearDemoSession() {
  clearStoredSession();
}

/**
 * Restore an existing demo session from localStorage without creating a new
 * account. Returns null if no stored session is available or it's invalid.
 */
export async function restoreDemoSession() {
  const stored = loadStoredSession();
  if (!stored) return null;
  const client = buildClient(stored);
  try {
    const res = await client.get("com.atproto.server.getSession");
    if (!res.ok) {
      clearStoredSession();
      return null;
    }
  } catch {
    clearStoredSession();
    return null;
  }
  return {
    did: stored.did,
    handle: stored.handle,
    name: stored.handle,
    client,
    pdsUri: stored.pdsUri,
    isDemo: true,
  };
}
