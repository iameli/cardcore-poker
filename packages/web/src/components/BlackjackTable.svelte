<script>
  import BlackjackSeat from "./BlackjackSeat.svelte";
  import Card from "./Card.svelte";

  let {
    players = {},
    playerOrder = [],
    handleMap = {},
    bankerDid = null,
    bankerCards = [],
    bankerTotal = 0,
    bankerSoft = false,
    minBet = 0,
    roundIndex = 0,
    currentPlayer = null,
    activeHand = null,
    ourPlayerId = "",
  } = $props();

  // Sprite row per seat position: top=row0(down), right=row1(left), left=row2(right), bottom=row3(up)
  const SPRITE_ROWS = { top: 0, right: 1, left: 2, bottom: 3 };

  // Same 8-slot ring as the poker table: local player rotated to
  // bottom-middle, everyone else spread clockwise (see PokerTable.svelte).
  const SLOTS_CW = [5, 4, 7, 0, 1, 2, 3, 6];
  const SLOT_POSITION = {
    0: "top",
    1: "top",
    2: "top",
    3: "right",
    4: "bottom",
    5: "bottom",
    6: "bottom",
    7: "left",
  };

  const slotAssignments = $derived.by(() => {
    const slots = new Array(8).fill(null);
    const N = playerOrder.length;
    if (N === 0) return slots;
    let myIdx = playerOrder.indexOf(ourPlayerId);
    if (myIdx < 0) myIdx = 0;
    for (let i = 0; i < N; i++) {
      const rel = (i - myIdx + N) % N;
      const cwIdx = Math.round((rel * 8) / N) % 8;
      slots[SLOTS_CW[cwIdx]] = playerOrder[i];
    }
    return slots;
  });

  function slotProps(slotIdx) {
    const did = slotAssignments[slotIdx];
    const p = did ? players[did] : null;
    const handle = did ? handleMap[did] || null : null;
    const position = SLOT_POSITION[slotIdx];
    return {
      id: did,
      player: p ? { ...p, name: handle || p.name } : null,
      did,
      spriteRow: SPRITE_ROWS[position] ?? 0,
    };
  }

  const bankerName = $derived(
    bankerDid ? handleMap[bankerDid] || players[bankerDid]?.name || "banker" : "banker",
  );
</script>

<div class="table-container">
  <div class="table-layout">
    <!-- Top row seats -->
    {#each [0, 1, 2] as i}
      {@const s = slotProps(i)}
      <div class="seat-area top">
        <BlackjackSeat
          player={s.player}
          seatIndex={i}
          isBanker={s.id != null && s.id === bankerDid}
          isCurrentPlayer={currentPlayer === s.id}
          activeHand={currentPlayer === s.id ? activeHand : null}
          isSelf={s.id === ourPlayerId}
          did={s.did}
          spriteRow={s.spriteRow}
        />
      </div>
    {/each}

    <!-- Left seat -->
    {#if true}
      {@const left = slotProps(7)}
      <div class="seat-area left">
        <BlackjackSeat
          player={left.player}
          seatIndex={7}
          isBanker={left.id != null && left.id === bankerDid}
          isCurrentPlayer={currentPlayer === left.id}
          activeHand={currentPlayer === left.id ? activeHand : null}
          isSelf={left.id === ourPlayerId}
          did={left.did}
          spriteRow={left.spriteRow}
        />
      </div>
    {/if}

    <!-- Table (center): the banker's face-up hand lives here -->
    <div class="blackjack-table">
      <div class="table-center">
        <div class="banker-line">
          <span class="banker-label">BANK</span>
          <span class="banker-name">{bankerName}</span>
        </div>
        <div class="banker-cards" data-testid="banker-cards">
          {#if bankerCards.length === 0}
            <div class="empty-slot"></div>
            <div class="empty-slot"></div>
          {:else}
            {#each bankerCards as card}
              <Card {card} />
            {/each}
          {/if}
        </div>
        {#if bankerCards.length > 0}
          <div class="banker-total" data-testid="banker-total">
            {bankerSoft ? "soft " : ""}{bankerTotal}
          </div>
        {/if}
        <div class="stakes-line">round {roundIndex + 1} · min bet {minBet}</div>
      </div>
    </div>

    <!-- Right seat -->
    {#if true}
      {@const right = slotProps(3)}
      <div class="seat-area right">
        <BlackjackSeat
          player={right.player}
          seatIndex={3}
          isBanker={right.id != null && right.id === bankerDid}
          isCurrentPlayer={currentPlayer === right.id}
          activeHand={currentPlayer === right.id ? activeHand : null}
          isSelf={right.id === ourPlayerId}
          did={right.did}
          spriteRow={right.spriteRow}
        />
      </div>
    {/if}

    <!-- Bottom row seats -->
    {#each [4, 5, 6] as i}
      {@const s = slotProps(i)}
      <div class="seat-area bottom">
        <BlackjackSeat
          player={s.player}
          seatIndex={i}
          isBanker={s.id != null && s.id === bankerDid}
          isCurrentPlayer={currentPlayer === s.id}
          activeHand={currentPlayer === s.id ? activeHand : null}
          isSelf={s.id === ourPlayerId}
          did={s.did}
          spriteRow={s.spriteRow}
        />
      </div>
    {/each}
  </div>
</div>

<style>
  .table-container {
    width: 100%;
    max-width: 900px;
    margin: 0 auto;
  }
  .table-layout {
    display: grid;
    grid-template-columns: auto 1fr auto;
    grid-template-rows: auto auto auto;
    gap: 0.5rem;
    align-items: center;
    justify-items: center;
  }
  .seat-area {
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .seat-area.top {
    grid-row: 1;
  }
  .seat-area.top:nth-child(1) {
    grid-column: 1;
  }
  .seat-area.top:nth-child(2) {
    grid-column: 2;
  }
  .seat-area.top:nth-child(3) {
    grid-column: 3;
  }

  .seat-area.left {
    grid-column: 1;
    grid-row: 2;
  }

  .seat-area.right {
    grid-column: 3;
    grid-row: 2;
  }

  .seat-area.bottom {
    grid-row: 3;
  }
  .seat-area.bottom:nth-child(7) {
    grid-column: 1;
  }
  .seat-area.bottom:nth-child(8) {
    grid-column: 2;
  }
  .seat-area.bottom:nth-child(9) {
    grid-column: 3;
  }

  .blackjack-table {
    grid-column: 2;
    grid-row: 2;
    position: relative;
    background: #ffffff;
    border: 6px solid #1a1a1a;
    border-radius: 0;
    width: 100%;
    min-width: 440px;
    min-height: 300px;
    box-shadow:
      0 0 0 3px #1a1a1a,
      8px 8px 0 #1a1a1a;
  }
  .table-center {
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.5rem;
    width: max-content;
    max-width: 95%;
  }
  .banker-line {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    background: #ffffff;
    border: 2px solid #1a1a1a;
    padding: 0.3rem 0.6rem;
    border-radius: 0;
  }
  .banker-label {
    font-size: 0.42rem;
    color: #ffffff;
    background: #1a1a1a;
    padding: 0 0.3rem;
    letter-spacing: 2px;
  }
  .banker-name {
    font-size: 0.45rem;
    color: #1a1a1a;
    max-width: 8rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .banker-cards {
    display: flex;
    gap: 0.3rem;
    flex-wrap: wrap;
    justify-content: center;
  }
  .banker-total {
    font-size: 0.5rem;
    color: #1a1a1a;
    border: 2px solid #1a1a1a;
    padding: 0 0.4rem;
    background: #ffffff;
  }
  .stakes-line {
    font-size: 0.4rem;
    color: #1a1a1a;
    opacity: 0.6;
    letter-spacing: 1px;
  }
  .empty-slot {
    width: 80px;
    height: 118px;
    border: 2px dashed #1a1a1a;
    border-radius: 0;
    opacity: 0.3;
  }
</style>
