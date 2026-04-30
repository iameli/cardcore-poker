/**
 * Poker game engine
 * Handles game state, betting rounds, hand evaluation, and turn logic.
 * Coordinates with the mental poker layer for card dealing.
 */

export const RANKS = ['A', '2', '3', '4', '5', '6', '7', '8', '9', '10', 'J', 'Q', 'K'];
export const SUITS = ['clubs', 'diamonds', 'hearts', 'spades'];

export const GAME_PHASES = {
  IDLE: 'idle',
  PREFLOP: 'preflop',
  FLOP: 'flop',
  TURN: 'turn',
  RIVER: 'river',
  SHOWDOWN: 'showdown',
};

export const ACTIONS = {
  FOLD: 'fold',
  CHECK: 'check',
  CALL: 'call',
  RAISE: 'raise',
  ALL_IN: 'allIn',
  DEAL: 'deal',
};

// ─── Hand Evaluation ─────────────────────────────────────────────

/**
 * Generate all combinations of k elements from an array.
 */
function combinations(arr, k) {
  if (k === 0) return [[]];
  if (arr.length < k) return [];
  const result = [];
  const first = arr[0];
  const rest = arr.slice(1);
  for (const combo of combinations(rest, k - 1)) {
    result.push([first, ...combo]);
  }
  for (const combo of combinations(rest, k)) {
    result.push(combo);
  }
  return result;
}

/**
 * Evaluate an exact 5-card hand.
 */
function evaluate5(cards) {
  const sorted = [...cards].sort((a, b) => a.index - b.index);
  const indices = sorted.map((c) => c.index);
  const suits = sorted.map((c) => c.suit);

  const isFlush = suits.every((s) => s === suits[0]);

  // Straight check
  let isStraight = true;
  for (let j = 0; j < 4; j++) {
    if (indices[j + 1] - indices[j] !== 1) {
      isStraight = false;
      break;
    }
  }
  // Ace-low straight (A,2,3,4,5)
  let aceLow = false;
  if (!isStraight) {
    const lowCheck = indices.map((i) => (i === 0 ? 13 : i)).sort((a, b) => a - b);
    aceLow = true;
    for (let j = 0; j < 4; j++) {
      if (lowCheck[j + 1] - lowCheck[j] !== 1) {
        aceLow = false;
        break;
      }
    }
  }
  const straight = isStraight || aceLow;
  const highCard = aceLow ? 3 : isStraight ? indices[4] : -1;

  // Rank counts
  const rankCounts = {};
  for (const c of cards) {
    rankCounts[c.index] = (rankCounts[c.index] || 0) + 1;
  }
  const counts = Object.values(rankCounts).sort((a, b) => b - a);

  // Determine hand rank
  let rank = 0;
  let name = 'HIGH CARD';
  let desc = `${RANKS[sorted[4].index]} High`;

  if (isFlush && straight && highCard === 12) {
    rank = 9;
    name = 'ROYAL FLUSH';
    desc = 'Royal Flush';
  } else if (isFlush && straight) {
    rank = 8;
    name = 'STRAIGHT FLUSH';
    desc = 'Straight Flush';
  } else if (counts[0] === 4) {
    rank = 7;
    name = 'FOUR OF A KIND';
    desc = 'Four of a Kind';
  } else if (counts[0] === 3 && counts[1] === 2) {
    rank = 6;
    name = 'FULL HOUSE';
    desc = 'Full House';
  } else if (isFlush) {
    rank = 5;
    name = 'FLUSH';
    desc = 'Flush';
  } else if (straight) {
    rank = 4;
    name = 'STRAIGHT';
    desc = 'Straight';
  } else if (counts[0] === 3) {
    rank = 3;
    name = 'THREE OF A KIND';
    desc = 'Three of a Kind';
  } else if (counts[0] === 2 && counts[1] === 2) {
    rank = 2;
    name = 'TWO PAIR';
    desc = 'Two Pair';
  } else if (counts[0] === 2) {
    rank = 1;
    name = 'ONE PAIR';
    desc = 'One Pair';
  }

  return { rank, name, desc };
}

/**
 * Evaluate the best 5-card poker hand from up to 7 cards.
 * For 6-7 cards, generates all C(n,5) combos and picks the best.
 */
export function evaluateHand(cards) {
  if (cards.length < 5) return { name: '', desc: '', rank: -1 };
  if (cards.length === 5) return evaluate5(cards);

  // 6 or 7 cards: try all 5-card combos
  const combos = combinations(cards, 5);
  let best = { rank: -1, name: '', desc: '' };
  for (const combo of combos) {
    const result = evaluate5(combo);
    if (result.rank > best.rank) best = result;
  }
  return best;
}

/**
 * Compare two hands - returns >0 if a wins, <0 if b wins, 0 if tie
 */
export function compareHands(handA, handB) {
  return handA.rank - handB.rank;
}

// ─── Actions ────────────────────────────────────────────────────

/**
 * Get available actions for a player
 * Returns actions array plus raise context (pot, min, max, quick amounts)
 */
export function getAvailableActions(gameState, playerId) {
  const player = gameState.players[playerId];
  if (!player || player.folded) return { actions: [], raise: null };

  const actions = [];
  const currentBet = gameState.currentBet || 0;
  const playerBet = player.bet || 0;
  const toCall = currentBet - playerBet;

  actions.push({ type: ACTIONS.FOLD, label: 'FOLD' });

  if (toCall <= 0) {
    actions.push({ type: ACTIONS.CHECK, label: 'CHECK' });
  } else {
    actions.push({ type: ACTIONS.CALL, label: `CALL ${toCall}`, amount: toCall });
  }

  const maxChips = player.chips;
  const minRaise = gameState.bigBlind || 2;

  if (maxChips > toCall) {
    const pot = gameState.pot || 0;
    const thirdPot = Math.floor(pot / 3);
    const halfPot = Math.floor(pot / 2);
    const fullPot = pot;

    const quickAmounts = [];
    if (thirdPot > minRaise) quickAmounts.push({ label: '1/3 POT', amount: thirdPot });
    if (halfPot > minRaise) quickAmounts.push({ label: '1/2 POT', amount: halfPot });
    if (fullPot > minRaise) quickAmounts.push({ label: 'POT', amount: fullPot });

    actions.push({ type: ACTIONS.RAISE, label: 'RAISE', amount: minRaise });

    actions.push({
      type: ACTIONS.ALL_IN,
      label: `ALL IN (${maxChips})`,
      amount: maxChips,
    });

    return {
      actions,
      raise: {
        min: minRaise,
        max: maxChips,
        pot,
        quickAmounts,
      },
    };
  }

  return { actions, raise: null };
}
