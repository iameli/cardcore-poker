<script>
  import Card from "./Card.svelte";

  let {
    player = null,
    seatIndex = 0,
    isBanker = false,
    isCurrentPlayer = false,
    activeHand = null,
    isSelf = false,
    did = null,
    spriteRow = 0,
  } = $props();

  const chips = $derived(player?.chips ?? 0);
  const name = $derived(player?.name ?? "Empty Seat");
  const empty = $derived(!player);
  const eliminated = $derived(player?.eliminated ?? false);
  const hands = $derived(player?.hands ?? []);

  function badgeFor(hand) {
    if (player?.surrendered) return "SURRENDER";
    if (hand.busted) return "BUST";
    if (hands.length === 1 && !hand.doubled && hand.cards.length === 2 && hand.total === 21) {
      return "BLACKJACK";
    }
    if (hand.doubled && hand.stood) return "DOUBLE";
    if (hand.stood) return "STAND";
    return "";
  }

  // Y offset into the sprite sheet for the selected row (always middle column)
  const spriteY = $derived(-spriteRow * 48);
  const spriteUrl = $derived(
    did ? `https://rpg.actor/api/sprite/normalized?did=${encodeURIComponent(did)}` : null,
  );

  let imgFailed = $state(false);
  $effect(() => {
    if (spriteUrl) {
      imgFailed = false;
    }
  });
</script>

<div
  class="seat"
  class:active={isCurrentPlayer}
  class:empty
  class:eliminated
  class:self={isSelf}
  data-testid={isBanker && !empty ? "banker-seat" : undefined}
>
  {#if !empty}
    <div class="player-avatar">
      {#if spriteUrl && !imgFailed}
        <img
          class="sprite"
          src={spriteUrl}
          alt="{name} character sprite"
          style:object-position="-48px {spriteY}px"
          onerror={() => (imgFailed = true)}
          onload={() => (imgFailed = false)}
        />
      {:else}
        <div class="placeholder-avatar">?</div>
      {/if}
    </div>

    <div class="hands">
      {#each hands as hand, hi}
        <div class="hand" class:turn={isCurrentPlayer && activeHand === hi && hands.length > 1}>
          <div class="cards-row">
            {#each hand.cards as card}
              <div class="card-overlap">
                <Card {card} />
              </div>
            {/each}
          </div>
          {#if hand.cards.length > 0}
            <div class="hand-meta">
              <span class="total">{hand.soft ? "soft " : ""}{hand.total}</span>
              {#if badgeFor(hand)}
                <span
                  class="badge"
                  class:bad={badgeFor(hand) === "BUST" || badgeFor(hand) === "SURRENDER"}
                  class:good={badgeFor(hand) === "BLACKJACK"}
                >
                  {badgeFor(hand)}
                </span>
              {/if}
            </div>
          {/if}
        </div>
      {/each}
    </div>

    <div class="player-info">
      <div class="player-name">
        {#if isSelf}*{/if}{name}
      </div>
      <div class="chips">{chips} chips</div>
      {#if isBanker}
        <div class="banker-badge">BANKER</div>
      {:else if player.wager > 0}
        <div class="wager">Wager: {player.wager}</div>
      {/if}
      {#if player.insurance > 0}
        <div class="insured">INSURED {player.insurance}</div>
      {/if}
      {#if eliminated}
        <div class="out-badge">OUT</div>
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
  .seat.eliminated {
    opacity: 0.4;
  }
  .hands {
    display: flex;
    gap: 0.4rem;
    align-items: flex-start;
  }
  .hand {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.15rem;
    padding: 2px;
    border: 2px solid transparent;
  }
  .hand.turn {
    border-color: #c0392b;
  }
  .cards-row {
    display: flex;
    min-height: 40px;
    align-items: center;
    justify-content: center;
    /* Hands grow card by card — overlap keeps long hands compact. */
  }
  .card-overlap {
    transform: scale(0.55);
    transform-origin: center left;
    margin-right: -64px;
    height: 66px;
    display: flex;
    align-items: center;
  }
  .card-overlap:last-child {
    margin-right: 12px;
  }
  .hand-meta {
    display: flex;
    align-items: center;
    gap: 0.3rem;
  }
  .total {
    font-size: 0.42rem;
    color: #1a1a1a;
    border: 2px solid #1a1a1a;
    padding: 0 0.25rem;
    background: #ffffff;
  }
  .badge {
    font-size: 0.38rem;
    letter-spacing: 1px;
    color: #1a1a1a;
  }
  .badge.bad {
    color: #c0392b;
  }
  .badge.good {
    color: #1a7a3a;
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
  .wager {
    font-size: 0.42rem;
    color: #c0392b;
  }
  .insured {
    font-size: 0.38rem;
    color: #1a7a3a;
    letter-spacing: 1px;
  }
  .banker-badge {
    font-size: 0.4rem;
    color: #ffffff;
    background: #1a1a1a;
    padding: 0 0.3rem;
    letter-spacing: 2px;
    margin-top: 2px;
    display: inline-block;
  }
  .out-badge {
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
