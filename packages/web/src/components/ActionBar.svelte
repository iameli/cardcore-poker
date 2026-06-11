<script>
  let { actions = [], raise = null, onAction, isOurTurn = false, placeholder = "" } = $props();

  let raiseAmount = $state(0);
  let showRaiseSlider = $state(false);

  $effect(() => {
    if (raise) {
      raiseAmount = raise.min;
    }
    showRaiseSlider = false;
  });

  function doAction(action) {
    if (!isOurTurn || !onAction) return;
    if (action.type === "raise") {
      if (!showRaiseSlider) {
        showRaiseSlider = true;
        return;
      }
      onAction({ ...action, amount: raiseAmount });
      showRaiseSlider = false;
    } else {
      onAction(action);
    }
  }
</script>

<!-- The bar always renders at the same height: buttons when it's our turn,
     an identically-sized placeholder line otherwise — so the layout doesn't
     jump every time the turn changes. (The raise panel below still expands;
     that's deliberate.) -->
<div class="action-bar">
  {#if actions.length > 0 && isOurTurn}
    {#each actions as action}
      {#if action.type === "raise"}
        <button class="action-btn raise" onclick={() => doAction(action)}>
          {showRaiseSlider ? `RAISE ${raiseAmount}` : "RAISE"}
        </button>
      {:else}
        <button class="action-btn {action.type}" onclick={() => doAction(action)}>
          {action.label}
        </button>
      {/if}
    {/each}
  {:else}
    <!-- A de-chromed disabled button: its metrics match the real buttons
         exactly, so the bar height never changes. -->
    <button class="action-btn placeholder" disabled tabindex="-1" data-testid="waiting-on">
      {placeholder || "\u00a0"}
    </button>
  {/if}
</div>

{#if showRaiseSlider && raise && isOurTurn}
  <div class="raise-panel">
    <div class="quick-btns">
      {#each raise.quickAmounts as q}
        <button
          class="quick-btn"
          onclick={() => (raiseAmount = q.amount)}
          class:active={raiseAmount === q.amount}
        >
          {q.label}<br /><span class="quick-val">{q.amount}</span>
        </button>
      {/each}
    </div>
    <div class="slider-row">
      <input type="range" min={raise.min} max={raise.max} bind:value={raiseAmount} />
      <span class="slider-val">{raiseAmount}</span>
    </div>
  </div>
{/if}

<style>
  .action-bar {
    display: flex;
    gap: 0.5rem;
    justify-content: center;
    padding: 0.75rem 0.75rem 0.25rem;
    flex-wrap: wrap;
  }
  .action-btn {
    /* Fixed height + flex centering, NOT vertical padding: stray glyphs that
       fall back to a non-pixel font (e.g. "…") would otherwise grow the line
       box and shift the whole layout. */
    height: 1.7rem;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0 1rem;
    border: 2px solid #1a1a1a;
    border-radius: 0;
    font-family: inherit;
    font-size: 0.45rem;
    cursor: pointer;
    letter-spacing: 1px;
    transition: all 0.1s;
    background: #ffffff;
    color: #1a1a1a;
    box-shadow: 3px 3px 0 #1a1a1a;
  }
  .action-btn.placeholder,
  .action-btn.placeholder:hover,
  .action-btn.placeholder:active {
    background: transparent;
    color: #1a1a1a;
    border-color: transparent;
    box-shadow: none;
    cursor: default;
    pointer-events: none;
    opacity: 0.5;
    transform: none;
  }
  .action-btn:hover {
    transform: translate(2px, 2px);
    box-shadow: 1px 1px 0 #1a1a1a;
  }
  .action-btn:active {
    transform: translate(3px, 3px);
    box-shadow: none;
  }
  .action-btn.fold {
    background: #ffffff;
    color: #1a1a1a;
  }
  .action-btn.check {
    background: #1a1a1a;
    color: #ffffff;
  }
  .action-btn.call {
    background: #1a1a1a;
    color: #ffffff;
  }
  .action-btn.raise {
    background: #c0392b;
    color: #ffffff;
  }
  .action-btn.allIn {
    background: #c0392b;
    color: #ffffff;
  }

  .raise-panel {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    padding: 0.5rem 0.75rem 0.75rem;
  }
  .quick-btns {
    display: flex;
    gap: 0.4rem;
    justify-content: center;
  }
  .quick-btn {
    padding: 0.35rem 0.5rem;
    border: 2px solid #1a1a1a;
    border-radius: 0;
    font-family: inherit;
    font-size: 0.42rem;
    cursor: pointer;
    background: #ffffff;
    color: #1a1a1a;
    box-shadow: 2px 2px 0 #1a1a1a;
    transition: all 0.1s;
    line-height: 1.3;
  }
  .quick-btn:hover {
    transform: translate(1px, 1px);
    box-shadow: 1px 1px 0 #1a1a1a;
  }
  .quick-btn.active {
    background: #c0392b;
    color: #ffffff;
  }
  .quick-val {
    font-size: 0.4rem;
    opacity: 0.7;
  }
  .slider-row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .slider-row input[type="range"] {
    flex: 1;
    -webkit-appearance: none;
    appearance: none;
    height: 4px;
    background: #1a1a1a;
    border: none;
    outline: none;
  }
  .slider-row input[type="range"]::-webkit-slider-thumb {
    -webkit-appearance: none;
    width: 14px;
    height: 14px;
    background: #c0392b;
    border: 2px solid #1a1a1a;
    border-radius: 0;
    cursor: pointer;
  }
  .slider-val {
    font-size: 0.4rem;
    color: #1a1a1a;
    min-width: 3rem;
    text-align: right;
  }
</style>
