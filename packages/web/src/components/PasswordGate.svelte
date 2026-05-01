<script>
  // Temporary soft-launch gate. Not security — just a "are you supposed to
  // be here" check while the project is invite-only. Remove this component
  // (and its mount in App.svelte) before going live properly.
  let { onUnlock } = $props();
  const PASSWORD = "pocketrockets";

  let input = $state("");
  let error = $state("");

  function submit() {
    if (input.trim() === PASSWORD) {
      try {
        localStorage.setItem("cardcore_unlocked", "1");
      } catch {}
      onUnlock();
    } else {
      error = "Wrong password.";
      input = "";
    }
  }
</script>

<div class="gate">
  <div class="card">
    <h1>CARDCORE POKER</h1>
    <p class="hint">Invite only for the moment.</p>
    <input
      type="password"
      placeholder="password"
      bind:value={input}
      onkeydown={(e) => e.key === "Enter" && submit()}
      autofocus
      data-testid="gate-password"
    />
    {#if error}<p class="error">{error}</p>{/if}
    <button class="btn" onclick={submit} data-testid="gate-submit">Enter</button>
  </div>
</div>

<style>
  .gate {
    min-height: 100vh;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 1rem;
    background: #ffffff;
  }
  .card {
    border: 3px solid #1a1a1a;
    box-shadow: 6px 6px 0 #1a1a1a;
    background: #ffffff;
    padding: 2rem;
    max-width: 420px;
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    text-align: center;
  }
  h1 {
    font-size: 1rem;
    color: #1a1a1a;
    letter-spacing: 1px;
    margin-bottom: 0.25rem;
  }
  .hint {
    font-size: 0.4rem;
    color: #1a1a1a;
    opacity: 0.6;
  }
  input {
    padding: 0.75rem;
    border: 2px solid #1a1a1a;
    border-radius: 0;
    background: #ffffff;
    color: #1a1a1a;
    font-family: inherit;
    font-size: 0.5rem;
    outline: none;
  }
  input:focus {
    border-color: #c0392b;
    box-shadow: 3px 3px 0 #c0392b;
  }
  .btn {
    padding: 0.7rem 1.2rem;
    border: 2px solid #1a1a1a;
    border-radius: 0;
    font-family: inherit;
    font-size: 0.45rem;
    cursor: pointer;
    letter-spacing: 1px;
    background: #c0392b;
    color: #ffffff;
    box-shadow: 3px 3px 0 #1a1a1a;
    transition: all 0.1s;
  }
  .btn:hover {
    transform: translate(2px, 2px);
    box-shadow: 1px 1px 0 #1a1a1a;
  }
  .btn:active {
    transform: translate(3px, 3px);
    box-shadow: none;
  }
  .error {
    color: #c0392b;
    font-size: 0.45rem;
  }
</style>
