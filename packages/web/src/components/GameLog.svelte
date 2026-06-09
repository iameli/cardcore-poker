<script>
  let { events = [] } = $props();

  let container = $state(null);
  // Pinned = following live: new entries auto-scroll into view. Scrolling up
  // unpins; the ↓ button re-pins.
  let pinned = $state(true);

  const AT_BOTTOM_SLOP = 8; // px of wiggle room before we count as "scrolled up"

  function onScroll() {
    if (!container) return;
    pinned =
      container.scrollHeight - container.scrollTop - container.clientHeight <= AT_BOTTOM_SLOP;
  }

  function jumpToLive() {
    if (!container) return;
    container.scrollTop = container.scrollHeight;
    pinned = true;
  }

  // Runs after the DOM updates with new entries — keep the bottom in view
  // unless the user is reading scrollback.
  $effect(() => {
    void events.length;
    if (pinned && container) {
      container.scrollTop = container.scrollHeight;
    }
  });
</script>

<div class="game-log">
  <div class="log-header">Game Log</div>
  <div class="log-entries" bind:this={container} onscroll={onScroll}>
    {#if events.length === 0}
      <div class="empty">Waiting for game to start...</div>
    {:else}
      {#each events as event, i}
        <div class="log-entry" class:fade={i < events.length - 4}>
          {event}
        </div>
      {/each}
    {/if}
  </div>
  {#if !pinned}
    <button
      class="jump-live"
      onclick={jumpToLive}
      data-testid="log-jump-live"
      title="Return to live"
    >
      ↓
    </button>
  {/if}
</div>

<style>
  .game-log {
    background: #ffffff;
    border: 2px solid #1a1a1a;
    border-radius: 0;
    overflow: hidden;
    max-height: 160px;
    display: flex;
    flex-direction: column;
    position: relative;
  }
  .log-header {
    font-size: 0.4rem;
    padding: 0.4rem 0.75rem;
    background: #1a1a1a;
    color: #ffffff;
    border-bottom: 2px solid #1a1a1a;
    letter-spacing: 1px;
  }
  .log-entries {
    overflow-y: auto;
    padding: 0.4rem;
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .log-entry {
    font-size: 0.42rem;
    color: #1a1a1a;
    padding: 2px 4px;
    border-radius: 0;
  }
  .log-entry.fade {
    opacity: 0.4;
  }
  .empty {
    font-size: 0.42rem;
    color: #1a1a1a;
    opacity: 0.4;
    text-align: center;
    padding: 0.5rem;
  }
  .jump-live {
    position: absolute;
    right: 0.5rem;
    bottom: 0.5rem;
    width: 1.1rem;
    height: 1.1rem;
    display: flex;
    align-items: center;
    justify-content: center;
    font-family: inherit;
    font-size: 0.5rem;
    background: #1a1a1a;
    color: #ffffff;
    border: 2px solid #1a1a1a;
    border-radius: 0;
    cursor: pointer;
    box-shadow: 2px 2px 0 rgba(26, 26, 26, 0.4);
  }
  .jump-live:hover {
    background: #c0392b;
    border-color: #c0392b;
  }
</style>
