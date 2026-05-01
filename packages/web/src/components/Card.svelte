<script>
  let { card = null, faceDown = false } = $props();

  const rankIndex = {
    A: 0,
    "2": 1,
    "3": 2,
    "4": 3,
    "5": 4,
    "6": 5,
    "7": 6,
    "8": 7,
    "9": 8,
    "10": 9,
    J: 10,
    Q: 11,
    K: 12,
  };

  const suitFile = {
    clubs: "/sprites/clubs.png",
    diamonds: "/sprites/diamonds.png",
    hearts: "/sprites/hearts.png",
    spades: "/sprites/spades.png",
  };

  const display = $derived(card && !faceDown ? card : null);
  const bgImage = $derived(display ? `url(${suitFile[display.suit]})` : "none");
  const bgX = $derived(display ? `${((rankIndex[display.rank] ?? 0) / 12) * 100}%` : "0%");
</script>

<div class="card" class:face-down={faceDown}>
  {#if faceDown || !card}
    <div class="card-back">
      <div class="back-pattern"></div>
    </div>
  {:else}
    <div class="card-face" style="background-image: {bgImage}; background-position: {bgX} 0;"></div>
  {/if}
</div>

<style>
  .card {
    width: 80px;
    height: 118px;
    border-radius: 0;
    position: relative;
    user-select: none;
    flex-shrink: 0;
  }
  .card-face {
    width: 100%;
    height: 100%;
    background-color: #ffffff;
    background-repeat: no-repeat;
    background-size: 1300% 100%;
    image-rendering: pixelated;
    border-radius: 0;
    border: 2px solid #1a1a1a;
    box-shadow: 3px 3px 0 #1a1a1a;
  }
  .card-back {
    width: 100%;
    height: 100%;
    background: #ffffff;
    border-radius: 0;
    border: 2px solid #1a1a1a;
    overflow: hidden;
    box-shadow: 3px 3px 0 #1a1a1a;
  }
  .back-pattern {
    width: 100%;
    height: 100%;
    background:
      repeating-linear-gradient(45deg, transparent, transparent 4px, #1a1a1a 4px, #1a1a1a 5px),
      repeating-linear-gradient(-45deg, transparent, transparent 4px, #1a1a1a 4px, #1a1a1a 5px);
    border-radius: 0;
  }
</style>
