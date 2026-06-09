<script>
  import Card from "./Card.svelte";

  let {
    player = null,
    seatIndex = 0,
    isCurrentPlayer = false,
    isSelf = false,
    did = null,
    spriteRow = 0,
    holeCards = [],
    communityCards = [],
    showCards = false,
  } = $props();

  const chips = $derived(player?.chips ?? 1000);
  const bet = $derived(player?.bet ?? 0);
  const folded = $derived(player?.folded ?? false);
  const name = $derived(player?.name ?? "Empty Seat");
  const empty = $derived(!player);

  const cardsToShow = $derived(showCards || isSelf ? holeCards : holeCards.map(() => null));

  // Y offset into the sprite sheet for the selected row (always middle column = col 1)
  const spriteY = $derived(-spriteRow * 48);

  const spriteUrl = $derived(
    did ? `https://rpg.actor/api/sprite/normalized?did=${encodeURIComponent(did)}` : null,
  );

  // Debug: log the DID → sprite mapping so you can verify the right DID is used
  $effect(() => {
    if (did) {
      console.log(`[PlayerSeat] ${name}: DID=${did} sprite=${spriteUrl}`);
    } else if (player) {
      console.log(`[PlayerSeat] ${name}: no DID (will show placeholder)`);
    }
  });

  let imgFailed = $state(false);

  // Reset error state when spriteUrl changes
  $effect(() => {
    if (spriteUrl) {
      imgFailed = false;
    }
  });

  function onSpriteError() {
    imgFailed = true;
  }

  function onSpriteLoad() {
    imgFailed = false;
  }
</script>

<div class="seat" class:active={isCurrentPlayer} class:empty class:folded class:self={isSelf}>
  {#if !empty}
    <div class="player-avatar">
      {#if spriteUrl && !imgFailed}
        <img
          class="sprite"
          src={spriteUrl}
          alt="{name} character sprite"
          style:object-position="-48px {spriteY}px"
          onerror={onSpriteError}
          onload={onSpriteLoad}
        />
      {:else}
        <div class="placeholder-avatar">?</div>
      {/if}
    </div>
    <div class="cards-row">
      {#each cardsToShow as card, i}
        <Card {card} faceDown={!card} />
      {/each}
    </div>
    <div class="player-info">
      <div class="player-name">
        {#if isSelf}*{/if}{name}
      </div>
      <div class="chips">{chips} chips</div>
      {#if bet > 0}
        <div class="bet">Bet: {bet}</div>
      {/if}
      {#if folded}
        <div class="folded-badge">FOLDED</div>
      {/if}
    </div>
    {#if isCurrentPlayer}
      <div class="turn-indicator">▼</div>
    {/if}
  {:else}
    <div class="empty-seat">Seat {seatIndex + 1}</div>
  {/if}
</div>

<style>
  .seat {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.25rem;
    padding: 0.5rem;
    border-radius: 0;
    border: 2px solid transparent;
    transition: all 0.1s;
    min-width: 90px;
    background: #ffffff;
  }
  .player-avatar {
    width: 48px;
    height: 48px;
    border: 2px solid #1a1a1a;
    border-radius: 0;
    overflow: hidden;
    image-rendering: pixelated;
    image-rendering: crisp-edges;
    background: #ffffff;
    flex-shrink: 0;
  }
  .sprite {
    width: 144px;
    height: 192px;
    object-fit: none;
    image-rendering: pixelated;
    image-rendering: crisp-edges;
  }
  .placeholder-avatar {
    width: 100%;
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 1rem;
    color: #1a1a1a;
    opacity: 0.3;
    background: repeating-linear-gradient(
      45deg,
      #1a1a1a 0px,
      #1a1a1a 2px,
      #ffffff 2px,
      #ffffff 6px
    );
  }
  .seat.active {
    border-color: #1a1a1a;
    box-shadow: 4px 4px 0 #c0392b;
  }
  .seat.self {
    border-color: #1a1a1a;
  }
  .seat.self.active {
    border-color: #1a1a1a;
    box-shadow: 4px 4px 0 #c0392b;
  }
  .seat.folded {
    opacity: 0.4;
  }
  .cards-row {
    display: flex;
    gap: 0.2rem;
    min-height: 40px;
    align-items: center;
    justify-content: center;
  }
  .player-info {
    text-align: center;
  }
  .player-name {
    font-size: 0.4rem;
    color: #1a1a1a;
    max-width: 90px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .chips {
    font-size: 0.42rem;
    color: #1a1a1a;
    opacity: 0.6;
  }
  .bet {
    font-size: 0.42rem;
    color: #c0392b;
  }
  .folded-badge {
    font-size: 0.4rem;
    color: #c0392b;
    margin-top: 2px;
    letter-spacing: 2px;
  }
  .turn-indicator {
    color: #c0392b;
    font-size: 0.55rem;
    animation: bounce 0.6s ease-in-out infinite alternate;
  }
  @keyframes bounce {
    from {
      transform: translateY(0);
    }
    to {
      transform: translateY(4px);
    }
  }
  .empty-seat {
    font-size: 0.4rem;
    color: #1a1a1a;
    opacity: 0.3;
    padding: 0.5rem;
    border: 2px dashed #1a1a1a;
    border-radius: 0;
    text-align: center;
    width: 60px;
  }
</style>
