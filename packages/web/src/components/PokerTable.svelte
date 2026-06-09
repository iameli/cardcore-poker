<script>
  import PlayerSeat from "./PlayerSeat.svelte";
  import Card from "./Card.svelte";

  let {
    players = [],
    playerOrder = [],
    playerDids = {},
    handleMap = {},
    holeCards = {},
    communityCards = [],
    pot = 0,
    currentPlayer = null,
    ourPlayerId = "",
    gamePhase = "idle",
    showAllCards = false,
  } = $props();

  const visibleCommunity = $derived(
    gamePhase === "preflop"
      ? []
      : gamePhase === "flop"
        ? communityCards.slice(0, 3)
        : gamePhase === "turn"
          ? communityCards.slice(0, 4)
          : gamePhase === "river" || gamePhase === "showdown"
            ? communityCards
            : [],
  );

  // Sprite row per seat position: top=row0(down), right=row1(left), left=row2(right), bottom=row3(up)
  const SPRITE_ROWS = { top: 0, right: 1, left: 2, bottom: 3 };

  // The 8 visual slots in clockwise order, starting at bottom-middle (where
  // the local player always sits). We rotate `playerOrder` so the local
  // player lands at SLOTS_CW[0], then spread the remaining players across
  // the remaining slots so they're roughly equidistant on the table.
  //   slots:    5 (bottom-mid) ← us
  //             4 (bottom-left)
  //             7 (left)
  //             0 (top-left)
  //             1 (top-mid)
  //             2 (top-right)
  //             3 (right)
  //             6 (bottom-right)
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

  // visual slot 0..7 → DID of the player sitting there (or null)
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
      hole: did ? holeCards[did] || [] : [],
      spriteRow: SPRITE_ROWS[position] ?? 0,
    };
  }
</script>

<div class="table-container">
  <div class="table-layout">
    <!-- Top row seats -->
    {#each [0, 1, 2] as i}
      {@const s = slotProps(i)}
      <div class="seat-area top">
        <PlayerSeat
          player={s.player}
          seatIndex={i}
          isCurrentPlayer={currentPlayer === s.id}
          isSelf={s.id === ourPlayerId}
          did={s.did}
          spriteRow={s.spriteRow}
          holeCards={s.hole}
          showCards={showAllCards}
        />
      </div>
    {/each}

    <!-- Left seat -->
    {#if true}
      {@const left = slotProps(7)}
      <div class="seat-area left">
        <PlayerSeat
          player={left.player}
          seatIndex={7}
          isCurrentPlayer={currentPlayer === left.id}
          isSelf={left.id === ourPlayerId}
          did={left.did}
          spriteRow={left.spriteRow}
          holeCards={left.hole}
          showCards={showAllCards}
        />
      </div>
    {/if}

    <!-- Table (center) -->
    <div class="poker-table">
      <div class="table-center">
        <div class="pot">
          <span class="pot-label">POT</span>
          <span class="pot-amount">{pot}</span>
        </div>
        <div class="community-cards">
          {#each Array(5) as _, i}
            <div class="community-slot">
              {#if visibleCommunity[i]}
                <Card card={visibleCommunity[i]} />
              {:else if gamePhase !== "idle" && gamePhase !== "preflop" && i >= visibleCommunity.length}
                <Card faceDown={true} />
              {:else}
                <div class="empty-slot"></div>
              {/if}
            </div>
          {/each}
        </div>
      </div>
    </div>

    <!-- Right seat -->
    {#if true}
      {@const right = slotProps(3)}
      <div class="seat-area right">
        <PlayerSeat
          player={right.player}
          seatIndex={3}
          isCurrentPlayer={currentPlayer === right.id}
          isSelf={right.id === ourPlayerId}
          did={right.did}
          spriteRow={right.spriteRow}
          holeCards={right.hole}
          showCards={showAllCards}
        />
      </div>
    {/if}

    <!-- Bottom row seats -->
    {#each [4, 5, 6] as i}
      {@const s = slotProps(i)}
      <div class="seat-area bottom">
        <PlayerSeat
          player={s.player}
          seatIndex={i}
          isCurrentPlayer={currentPlayer === s.id}
          isSelf={s.id === ourPlayerId}
          did={s.did}
          spriteRow={s.spriteRow}
          holeCards={s.hole}
          showCards={showAllCards}
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

  .poker-table {
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
  }
  .pot {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    background: #ffffff;
    border: 2px solid #1a1a1a;
    padding: 0.3rem 0.6rem;
    border-radius: 0;
  }
  .pot-label {
    font-size: 0.42rem;
    color: #c0392b;
    letter-spacing: 2px;
  }
  .pot-amount {
    font-size: 0.55rem;
    color: #1a1a1a;
  }
  .community-cards {
    display: flex;
    gap: 0.3rem;
  }
  .community-slot {
    width: 80px;
    height: 118px;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .empty-slot {
    width: 80px;
    height: 118px;
    border: 2px dashed #1a1a1a;
    border-radius: 0;
    opacity: 0.3;
  }
</style>
