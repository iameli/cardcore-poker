/**
 * AT Protocol OAuth integration via @atcute/oauth-browser-client
 *
 * This lightweight library works on localhost / 127.0.0.1 with HTTP —
 * no HTTPS required for local development.
 */
import {
  configureOAuth,
  createAuthorizationUrl,
  finalizeAuthorization,
  getSession,
  deleteStoredSession,
  listStoredSessions,
} from '@atcute/oauth-browser-client';

const SCOPE = 'atproto transition:generic';

/**
 * client_id for development uses a special loopback format that tells the PDS
 * not to fetch metadata from our unreachable 127.0.0.1 address.
 * @see https://github.com/mary-ext/atcute — "local development with Vite"
 */
const DEV_REDIRECT_URI = 'http://127.0.0.1:5173/oauth-callback';

const CLIENT_ID = import.meta.env.DEV
  ? `http://localhost?redirect_uri=${encodeURIComponent(DEV_REDIRECT_URI)}&scope=${encodeURIComponent(SCOPE)}`
  : import.meta.env.VITE_OAUTH_CLIENT_ID;

const REDIRECT_URI = import.meta.env.DEV
  ? DEV_REDIRECT_URI
  : import.meta.env.VITE_OAUTH_REDIRECT_URI;

/**
 * Resolve a DID to its Personal Data Server (PDS) endpoint.
 * Handles did:plc (via PLC directory) and did:web (via well-known).
 */
async function resolveDidToPds(did) {
  if (did.startsWith('did:plc:')) {
    const res = await fetch(`https://plc.directory/${encodeURIComponent(did)}`);
    if (!res.ok) throw new Error(`PLC directory returned ${res.status}`);
    const doc = await res.json();
    const pdsService = doc.service?.find(
      (s) => s.type === 'AtprotoPersonalDataServer',
    );
    if (!pdsService?.serviceEndpoint) {
      throw new Error('No AtprotoPersonalDataServer service in DID document');
    }
    return pdsService.serviceEndpoint;
  }

  if (did.startsWith('did:web:')) {
    const domain = did.replace('did:web:', '').replace(/%3A/g, ':');
    const res = await fetch(`https://${domain}/.well-known/did.json`);
    if (!res.ok) throw new Error(`DID document for ${did} returned ${res.status}`);
    const doc = await res.json();
    const pdsService = doc.service?.find(
      (s) => s.type === 'AtprotoPersonalDataServer',
    );
    if (!pdsService?.serviceEndpoint) {
      throw new Error('No AtprotoPersonalDataServer service in DID document');
    }
    return pdsService.serviceEndpoint;
  }

  throw new Error(`Unsupported DID method: ${did}`);
}

let _configured = false;
let _configuring = false;

function ensureConfigured() {
  if (_configured) return;
  if (_configuring) return;
  _configuring = true;

  try {
    configureOAuth({
      metadata: {
        client_id: CLIENT_ID,
        redirect_uri: REDIRECT_URI,
      },
      // Resolve handle or DID → DID + PDS.
      // Called with a handle during sign-in, and with a DID after OAuth callback.
      identityResolver: {
        async resolve(ident) {
          let did;

          if (ident.startsWith('did:')) {
            // Already a DID — skip handle resolution
            did = ident;
          } else {
            // Resolve handle to DID
            const res = await fetch(
              `https://public.api.bsky.app/xrpc/com.atproto.identity.resolveHandle?handle=${encodeURIComponent(ident)}`,
            );
            if (!res.ok) throw new Error('Handle not found');
            ({ did } = await res.json());
          }

          // Resolve DID to DID document to discover the PDS endpoint
          const pds = await resolveDidToPds(did);
          if (!pds) throw new Error('Could not discover PDS for ' + did);

          return { did, pds };
        },
      },
      storageName: 'cardcore-oauth',
    });
    _configured = true;
  } catch (err) {
    console.error('Failed to configure OAuth:', err);
    _configuring = false;
    throw err;
  }
}

/**
 * Sign in with an AT Protocol handle.
 * Redirects the browser to the PDS for authorization.
 */
export async function signIn(handle) {
  ensureConfigured();

  const authUrl = await createAuthorizationUrl({
    target: { type: 'account', identifier: handle.trim() },
    scope: SCOPE,
  });

  // Redirect the browser to the authorization URL
  window.location.href = authUrl.toString();
}

/**
 * Resolve a DID (or handle) to a handle via Slingshot's cached identity resolver.
 */
export async function resolveDidToHandle(did) {
  try {
    const res = await fetch(
      `https://slingshot.microcosm.blue/xrpc/blue.microcosm.identity.resolveMiniDoc?identifier=${encodeURIComponent(did)}`,
    );
    if (res.ok) {
      const data = await res.json();
      return data.handle || '';
    }
  } catch {
    // best effort
  }
  return '';
}

/**
 * Fetch a user's profile (avatar, display name, etc).
 */
export async function fetchProfile(did) {
  try {
    const res = await fetch(
      `https://public.api.bsky.app/xrpc/app.bsky.actor.profile.get?actor=${encodeURIComponent(did)}`,
    );
    if (res.ok) {
      const data = await res.json();
      return {
        avatar: data.avatar || null,
        displayName: data.displayName || '',
        handle: data.handle || '',
      };
    }
  } catch {
    // best effort
  }
  return null;
}

/**
 * Batch resolve DIDs to handles. Returns a Map of did → handle.
 */
export async function resolveHandles(dids) {
  const unique = [...new Set(dids.filter(Boolean))];
  const results = new Map();
  await Promise.all(unique.map(async (did) => {
    const handle = await resolveDidToHandle(did);
    if (handle) results.set(did, handle);
  }));
  return results;
}

/**
 * Handle the OAuth callback after the user returns from authorization.
 * Returns the session if successful, null otherwise.
 */
export async function handleCallback() {
  ensureConfigured();

  try {
    const params = new URLSearchParams(window.location.hash.slice(1) || window.location.search);
    const { session } = await finalizeAuthorization(params);

    const handle = await resolveDidToHandle(session.info.sub);

    return {
      did: session.info.sub,
      handle,
      name: handle || session.info.sub,
      session,
    };
  } catch (err) {
    console.error('OAuth callback failed:', err);
    return null;
  }
}

/**
 * Get the stored session for the given DID (or the most recent session).
 */
export async function getStoredSession(did) {
  ensureConfigured();

  try {
    if (did) {
      const session = await getSession(did, { allowStale: true });
      const handle = await resolveDidToHandle(session.info.sub);
      return {
        did: session.info.sub,
        handle,
        name: handle || session.info.sub,
        session,
      };
    }

    const sessions = listStoredSessions();
    if (sessions.length > 0) {
      const sub = sessions[0];
      return getStoredSession(sub);
    }
    return null;
  } catch {
    return null;
  }
}

/**
 * Sign out and revoke tokens.
 */
export async function signOut() {
  ensureConfigured();

  try {
    const sessions = listStoredSessions();
    for (const sub of sessions) {
      try {
        const session = await getSession(sub, { allowStale: true });
        if (session) {
          deleteStoredSession(sub);
        }
      } catch {
        deleteStoredSession(sub);
      }
    }
  } catch {
    // Best effort
  }
}

/**
 * Demo auth using localStorage identity.
 * Works everywhere — no account or HTTPS needed.
 */
export function getDemoIdentity() {
  let identity = localStorage.getItem('cardcore_demo_identity');
  if (!identity) {
    const id = `@demo-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}.bsky.social`;
    identity = JSON.stringify({
      did: `did:plc:${id}`,
      handle: id,
      name: id.slice(1, 9),
    });
    localStorage.setItem('cardcore_demo_identity', identity);
  }
  return JSON.parse(identity);
}
