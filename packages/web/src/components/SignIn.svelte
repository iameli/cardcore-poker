<script>
  import { signIn } from "../lib/atproto.js";
  import { getOrCreateDemoSession } from "../lib/demo-pds.js";

  let { onSignIn } = $props();

  let handle = $state("");
  let loading = $state(false);
  let error = $state("");

  async function doSignIn() {
    if (!handle.trim()) {
      error = "Enter your AT Protocol handle";
      return;
    }
    loading = true;
    error = "";
    try {
      await signIn(handle.trim());
      // signIn redirects the browser — we never reach here on success
    } catch (e) {
      const msg = e?.message || e?.toString() || "Unknown error";
      error = msg;
      loading = false;
    }
  }

  async function doDemoSignIn() {
    loading = true;
    error = "";
    try {
      const session = await getOrCreateDemoSession();
      onSignIn(session);
    } catch (e) {
      error = "Demo signin failed: " + (e?.message || e);
      loading = false;
    }
  }
</script>

<div class="signin">
  <div class="card">
    <div class="logo">
      <div class="logo-suit" style="background-position: -18px 0;"></div>
      <div class="logo-suit" style="background-position: -36px 0;"></div>
      <h1>CARDCORE POKER</h1>
      <div class="logo-suit" style="background-position: 0px 0;"></div>
      <div class="logo-suit" style="background-position: -54px 0;"></div>
    </div>

    <p class="subtitle">Mental Poker &bull; Texas Hold'em</p>

    <div class="form">
      <label for="handle">AT Protocol Handle</label>
      <input
        id="handle"
        type="text"
        placeholder="you.bsky.social"
        bind:value={handle}
        onkeydown={(e) => e.key === "Enter" && doSignIn()}
        disabled={loading}
      />
      {#if error}
        <p class="error">{error}</p>
      {/if}

      <button class="btn primary" onclick={doSignIn} disabled={loading}>
        {loading ? "Signing in..." : "Sign In with AT Protocol"}
      </button>

      <div class="divider">
        <span>or</span>
      </div>

      <button class="btn demo" onclick={doDemoSignIn}> Play in Demo Mode </button>

      <p class="hint">Demo mode uses a local identity &mdash; no account needed.</p>
    </div>
  </div>
</div>

<style>
  .signin {
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 100vh;
    padding: 1rem;
    background: #ffffff;
  }
  .card {
    background: #ffffff;
    border: 3px solid #1a1a1a;
    border-radius: 0;
    padding: 3rem 2rem;
    max-width: 460px;
    width: 100%;
    text-align: center;
    box-shadow: 6px 6px 0 #1a1a1a;
  }
  .logo {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0.35rem;
    margin-bottom: 0.5rem;
  }
  .logo h1 {
    font-size: 1rem;
    color: #1a1a1a;
    letter-spacing: 1px;
    line-height: 1.6;
  }
  .logo-suit {
    width: 18px;
    height: 22px;
    background-image: url("/sprites/components.png");
    background-repeat: no-repeat;
    background-size: auto 100%;
    image-rendering: pixelated;
  }
  .subtitle {
    font-size: 0.5rem;
    color: #1a1a1a;
    margin-bottom: 2rem;
    letter-spacing: 1px;
    opacity: 0.7;
  }
  .form {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }
  label {
    font-size: 0.5rem;
    color: #1a1a1a;
    text-align: left;
  }
  input {
    padding: 0.75rem;
    border: 2px solid #1a1a1a;
    border-radius: 0;
    background: #ffffff;
    color: #1a1a1a;
    font-family: inherit;
    font-size: 0.55rem;
    outline: none;
  }
  input:focus {
    border-color: #c0392b;
    box-shadow: 3px 3px 0 #c0392b;
  }
  .error {
    color: #c0392b;
    font-size: 0.45rem;
  }
  .btn {
    padding: 0.75rem 1.5rem;
    border: 2px solid #1a1a1a;
    border-radius: 0;
    font-family: inherit;
    font-size: 0.55rem;
    cursor: pointer;
    letter-spacing: 1px;
    background: #ffffff;
    color: #1a1a1a;
    box-shadow: 3px 3px 0 #1a1a1a;
    transition: all 0.1s;
  }
  .btn:hover:not(:disabled) {
    transform: translate(2px, 2px);
    box-shadow: 1px 1px 0 #1a1a1a;
  }
  .btn:active:not(:disabled) {
    transform: translate(3px, 3px);
    box-shadow: none;
  }
  .btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
    transform: none;
    box-shadow: 3px 3px 0 #1a1a1a;
  }
  .primary {
    background: #c0392b;
    color: #ffffff;
    border-color: #1a1a1a;
  }
  .demo {
    background: #ffffff;
    color: #1a1a1a;
    border-color: #1a1a1a;
  }
  .divider {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin: 0.5rem 0;
  }
  .divider::before,
  .divider::after {
    content: "";
    flex: 1;
    height: 2px;
    background: #1a1a1a;
  }
  .divider span {
    font-size: 0.45rem;
    color: #1a1a1a;
    opacity: 0.6;
  }
  .hint {
    font-size: 0.4rem;
    color: #1a1a1a;
    line-height: 1.6;
    opacity: 0.5;
  }
</style>
