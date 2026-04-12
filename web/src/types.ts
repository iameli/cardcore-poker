export type GameEvent =
  | {
      type: "tableCreated";
      players: string[];
      startingChips: number;
      smallBlind: number;
    }
  | { type: "setupProgress"; phase: string; player: number }
  | {
      type: "blindsPosted";
      smallBlindPlayer: number;
      smallBlindAmount: number;
      bigBlindPlayer: number;
      bigBlindAmount: number;
    }
  | { type: "holeCardsDealt"; player: number; cards: string[] }
  | { type: "communityDealt"; street: string; cards: string[] }
  | {
      type: "playerBet";
      player: number;
      action: string;
      amount: number | null;
      pot: number;
    }
  | { type: "playerFolded"; player: number }
  | {
      type: "showdownReveal";
      player: number;
      cards: string[];
      hand_description: string;
    }
  | {
      type: "winner";
      players: number[];
      amount: number;
      hand_description: string;
    }
  | { type: "winByFold"; player: number; amount: number }
  | { type: "seedsVerified" }
  | { type: "gameOver"; chips: number[] };

export interface PlayerState {
  did: string;
  holeCards: string[];
  folded: boolean;
  lastAction: string | null;
}

export interface TableState {
  players: PlayerState[];
  communityCards: string[];
  street: string;
  pot: number;
  currentEvent: number;
  showdown: boolean;
}

export function buildTableState(events: GameEvent[], upTo: number): TableState {
  const state: TableState = {
    players: [],
    communityCards: [],
    street: "preflop",
    pot: 0,
    currentEvent: upTo,
    showdown: false,
  };

  for (let i = 0; i <= upTo && i < events.length; i++) {
    const e = events[i];
    switch (e.type) {
      case "tableCreated":
        state.players = e.players.map((did) => ({
          did,
          holeCards: [],
          folded: false,
          lastAction: null,
        }));
        break;
      case "holeCardsDealt":
        if (state.players[e.player]) {
          state.players[e.player].holeCards = e.cards;
        }
        break;
      case "communityDealt":
        state.communityCards.push(...e.cards);
        state.street = e.street;
        break;
      case "playerBet":
        if (state.players[e.player]) {
          state.players[e.player].lastAction = e.action;
          state.pot = e.pot;
        }
        break;
      case "playerFolded":
        if (state.players[e.player]) {
          state.players[e.player].folded = true;
          state.players[e.player].lastAction = "fold";
        }
        break;
      case "showdownReveal":
        state.showdown = true;
        if (state.players[e.player]) {
          state.players[e.player].holeCards = e.cards;
          state.players[e.player].lastAction = e.hand_description;
        }
        break;
    }
  }

  return state;
}
