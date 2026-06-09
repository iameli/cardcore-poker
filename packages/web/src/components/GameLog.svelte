<script>
  let { events = [] } = $props();
</script>

<div class="game-log">
  <div class="log-header">Game Log</div>
  <div class="log-entries">
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
</style>
