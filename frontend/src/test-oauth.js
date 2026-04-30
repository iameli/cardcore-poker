/**
 * Minimal test: import OAuth functions one by one to find the breakage.
 * Run from browser console: import('/src/test-oauth.js')
 */

const results = [];

async function test(name, fn) {
  try {
    const val = await fn();
    results.push({ name, ok: true, type: typeof val, keys: val ? Object.keys(val).slice(0, 5) : null });
    console.log(`✅ ${name} — OK (${typeof val})`);
  } catch (e) {
    results.push({ name, ok: false, error: e.message, stack: e.stack?.split('\n').slice(0, 3).join('\n') });
    console.error(`❌ ${name} — FAILED:`, e.message);
    console.error(e.stack);
  }
}

async function run() {
  console.log('=== OAuth Debug Tests ===');
  console.log('location:', window.location.href);
  console.log('origin:', window.location.origin);
  console.log('crypto:', typeof crypto, typeof crypto?.subtle);
  console.log('localStorage:', typeof localStorage);

  // Test 1: raw dynamic import of the package
  await test('import @atcute/oauth-browser-client', () =>
    import('@atcute/oauth-browser-client')
  );

  // Test 2: named imports
  await test('named imports', async () => {
    const mod = await import('@atcute/oauth-browser-client');
    return {
      configureOAuth: typeof mod.configureOAuth,
      createAuthorizationUrl: typeof mod.createAuthorizationUrl,
      finalizeAuthorization: typeof mod.finalizeAuthorization,
      getSession: typeof mod.getSession,
      deleteStoredSession: typeof mod.deleteStoredSession,
      listStoredSessions: typeof mod.listStoredSessions,
    };
  });

  // Test 3: configureOAuth call (does it throw?)
  await test('configureOAuth()', async () => {
    const { configureOAuth } = await import('@atcute/oauth-browser-client');
    configureOAuth({
      metadata: {
        client_id: `${window.location.origin}/oauth-callback`,
        redirect_uri: `${window.location.origin}/oauth-callback`,
      },
      identityResolver: {
        async resolve(handle) {
          const res = await fetch(
            `https://public.api.bsky.app/xrpc/com.atproto.identity.resolveHandle?handle=${encodeURIComponent(handle)}`
          );
          if (!res.ok) throw new Error('Handle not found');
          const body = await res.json();
          return { did: body.did };
        },
      },
      storageName: 'test-cardcore-oauth',
    });
    return 'configured';
  });

  // Test 4: our atproto.js module
  await test('import ../lib/atproto.js', () =>
    import('../lib/atproto.js')
  );

  // Test 5: try signIn (won't redirect, just build the URL)
  await test('createAuthorizationUrl', async () => {
    const { createAuthorizationUrl } = await import('@atcute/oauth-browser-client');
    const url = await createAuthorizationUrl({
      target: { type: 'account', identifier: 'test.bsky.social' },
      scope: 'atproto transition:generic',
    });
    return url.toString();
  });

  console.log('=== Results ===');
  console.table(results);
  return results;
}

export { run, test, results };
