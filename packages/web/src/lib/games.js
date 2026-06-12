/**
 * Game registry — every game cardcore can play.
 *
 * Each entry carries the AT Protocol collections its records live in, the
 * lobby defaults, the label for its stakes field, and the Svelte component
 * that renders a running game. Routing keys off the table collection in an
 * `at://` deep link, so poker and blackjack rooms coexist without touching
 * each other's screens.
 */
import GameRoom from "../components/GameRoom.svelte";
import BlackjackRoom from "../components/BlackjackRoom.svelte";
import { BLACKJACK_LEXICONS, LEXICONS } from "./atproto-publisher.js";

export const GAMES = {
  poker: {
    id: "poker",
    label: "Poker",
    description: "Texas Hold'em",
    tableCollection: LEXICONS.TABLE,
    actionCollection: LEXICONS.ACTION,
    stakesField: "smallBlind",
    stakesLabel: "small blind",
    defaults: { startingChips: 1000, stakes: 10 },
    roomComponent: GameRoom,
  },
  blackjack: {
    id: "blackjack",
    label: "Blackjack",
    description: "Rotating banker, no hole card",
    tableCollection: BLACKJACK_LEXICONS.TABLE,
    actionCollection: BLACKJACK_LEXICONS.ACTION,
    stakesField: "minBet",
    stakesLabel: "min bet",
    defaults: { startingChips: 1000, stakes: 10 },
    roomComponent: BlackjackRoom,
  },
};

export const DEFAULT_GAME = GAMES.poker;

/** The game whose table collection matches `nsid`, or null. */
export function gameForCollection(nsid) {
  return Object.values(GAMES).find((g) => g.tableCollection === nsid) ?? null;
}

/**
 * Resolve the game for an `at://did/collection/rkey` table URI. Unknown
 * collections fall back to poker (fetchTableRecord rejects them anyway).
 */
export function gameForTableUri(uri) {
  const collection = uri?.split("/")[3];
  return gameForCollection(collection) ?? DEFAULT_GAME;
}
