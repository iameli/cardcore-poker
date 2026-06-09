<script>
  let { session, uri, onStartGame, onLeaveRoom } = $props();

  let copied = $state(false);

  function copyUri() {
    if (uri) {
      navigator.clipboard.writeText(uri);
      copied = true;
      setTimeout(() => {
        copied = false;
      }, 2000);
    }
  }

  const tid = $derived(uri ? uri.split("/").pop() : "");
  const playerName = $derived(session?.handle || session?.name || "Player");
</script>

<div class="room-lobby">
  <header>
    <div class="user-info">
      <span class="name">{playerName}</span>
      <span class="did" title={session?.did}>
        {session?.did?.slice(0, 12)}…{session?.did?.slice(-6)}
      </span>
    </div>
    <button class="btn logout" onclick={onLeaveRoom}>Leave</button>
  </header>

  <div class="content">
    <h2>Room Waiting for Players</h2>

    <section class="card">
      <h3>Share This Link</h3>
      <p class="hint">Send this to other players so they can join:</p>
      <div class="uri-container" data-testid="copy-table-uri">
        <code>{tid}</code>
        <button class="btn secondary" onclick={copyUri} data-testid="copy-uri-button">
          {copied ? "Copied!" : "Copy"}
        </button>
      </div>
    </section>

    <div class="actions">
      <button class="btn primary" onclick={onStartGame} data-testid="start-game">
        Start Game
      </button>
    </div>
  </div>
</div>

<style>
  .room-lobby {
    min-height: 100vh;
    display: flex;
    flex-direction: column;
    background: #ffffff;
  }
  header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 1rem 1.5rem;
    background: #ffffff;
    border-bottom: 3px solid #1a1a1a;
  }
  .user-info {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
  }
  .name {
    font-size: 0.5rem;
    color: #1a1a1a;
  }
  .did {
    font-size: 0.32rem;
    color: #1a1a1a;
    opacity: 0.5;
  }
  .content {
    flex: 1;
    max-width: 600px;
    width: 100%;
    margin: 0 auto;
    padding: 2rem 1rem;
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }
  h2 {
    text-align: center;
    font-size: 1.7rem;
    margin-bottom: 1rem;
    color: #1a1a1a;
    letter-spacing: 2px;
  }
  h3 {
    font-size: 0.55rem;
    color: #1a1a1a;
    margin-bottom: 0.5rem;
  }
  .card {
    border: 3px solid #1a1a1a;
    box-shadow: 6px 6px 0 #1a1a1a;
    padding: 1rem;
    background: #ffffff;
  }
  .hint {
    font-size: 0.4rem;
    color: #1a1a1a;
    opacity: 0.6;
    margin-bottom: 0.75rem;
  }
  .uri-container {
    display: flex;
    gap: 0.5rem;
    align-items: center;
  }
  code {
    flex: 1;
    padding: 0.7rem;
    background: #f5f5f5;
    border: 2px solid #1a1a1a;
    font-family: monospace;
    font-size: 0.35rem;
    color: #1a1a1a;
    word-break: break-all;
    overflow-wrap: break-word;
  }
  .btn {
    padding: 0.7rem 1.2rem;
    border: 2px solid #1a1a1a;
    border-radius: 0;
    font-family: inherit;
    font-size: 0.45rem;
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
  }
  .secondary {
    background: #1a1a1a;
    color: #ffffff;
    white-space: nowrap;
    flex-shrink: 0;
  }
  .logout {
    font-size: 0.4rem;
    padding: 0.4rem 0.8rem;
  }
  .logout:hover {
    background: #c0392b;
    color: #ffffff;
  }
  .actions {
    display: flex;
    justify-content: center;
    gap: 1rem;
    margin-top: 1rem;
  }
</style>
