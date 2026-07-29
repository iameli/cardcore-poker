<script>
  let { events = [] } = $props();

  let container = $state(null);
  // Pinned = following live: new entries auto-scroll into view. Scrolling up
  // unpins; the ↓ button re-pins.
  let pinned = $state(true);
  // Expanded protocol groups, keyed by the id of the group's first entry —
  // stable while new steps append to the group's tail.
  let expanded = $state(new Set());

  const AT_BOTTOM_SLOP = 8; // px of wiggle room before we count as "scrolled up"

  // Runs of noninteractive protocol steps (commits, shuffles, reveals) fold
  // into a single collapsible row so the human story stays readable — but
  // every step remains inspectable behind the fold.
  const items = $derived.by(() => {
    const out = [];
    for (const ev of events) {
      const last = out[out.length - 1];
      if (ev.protocol) {
        if (last?.type === "group") last.events.push(ev);
        else out.push({ type: "group", key: `g${ev.id}`, events: [ev] });
      } else {
        out.push({ type: "entry", key: `e${ev.id}`, event: ev });
      }
    }
    return out;
  });

  function toggleGroup(key) {
    const next = new Set(expanded);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    expanded = next;
  }

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
    void expanded;
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
      {#each items as item, i (item.key)}
        {#if item.type === "entry"}
          <div class="log-entry" class:fade={i < items.length - 4}>
            {item.event.text}
          </div>
        {:else}
          <button
            class="log-fold"
            class:fade={i < items.length - 4}
            data-testid="log-fold"
            onclick={() => toggleGroup(item.key)}
          >
            {expanded.has(item.key) ? "▾" : "▸"}
            {item.events.length} protocol step{item.events.length === 1 ? "" : "s"}
          </button>
          {#if expanded.has(item.key)}
            {#each item.events as ev (ev.id)}
              <div class="log-entry protocol" data-testid="log-protocol-entry">
                {ev.text}
              </div>
            {/each}
          {/if}
        {/if}
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
  .log-entry.protocol {
    opacity: 0.6;
    padding-left: 0.8rem;
  }
  .log-fold {
    font-family: inherit;
    font-size: 0.42rem;
    color: #1a1a1a;
    opacity: 0.65;
    padding: 2px 4px;
    background: none;
    border: none;
    border-radius: 0;
    cursor: pointer;
    text-align: left;
    letter-spacing: 1px;
  }
  .log-fold:hover {
    opacity: 1;
    color: #c0392b;
  }
  .log-fold.fade {
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
