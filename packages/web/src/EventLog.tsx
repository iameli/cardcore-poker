import { View, Text, StyleSheet, ScrollView } from "react-native";
import type { GameEvent } from "./types";

function eventLabel(e: GameEvent): string {
  switch (e.type) {
    case "tableCreated":
      return `Table: ${e.players.length} players, ${e.startingChips} chips`;
    case "setupProgress":
      return `Setup: ${e.phase} (player ${e.player})`;
    case "holeCardsDealt":
      return `Player ${e.player}: dealt ${e.cards.join(" ")}`;
    case "communityDealt":
      return `${e.street}: ${e.cards.join(" ")}`;
    case "playerBet":
      return `Player ${e.player}: ${e.action}${e.amount ? ` (${e.amount})` : ""}`;
    case "playerFolded":
      return `Player ${e.player}: fold`;
    case "showdownReveal":
      return `Player ${e.player}: ${e.cards.join(" ")} — ${e.hand_description}`;
    case "winner":
      return `Winner: Player ${e.players.join(", ")} — ${e.hand_description} (${e.amount})`;
    case "winByFold":
      return `Player ${e.player} wins ${e.amount} (fold)`;
    case "seedsVerified":
      return "Seeds verified ✓";
    case "gameOver":
      return "Game over";
    default:
      return JSON.stringify(e);
  }
}

export function EventLog({
  events,
  currentIndex,
  onSelect,
}: {
  events: GameEvent[];
  currentIndex: number;
  onSelect: (index: number) => void;
}) {
  return (
    <ScrollView style={styles.container} testID="event-log">
      {events.map((e, i) => (
        <Text
          key={i}
          style={[
            styles.event,
            i === currentIndex && styles.current,
            i > currentIndex && styles.future,
          ]}
          onPress={() => onSelect(i)}
          testID={`event-${i}`}
        >
          {i}. {eventLabel(e)}
        </Text>
      ))}
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  container: {
    maxHeight: 300,
    backgroundColor: "#161b22",
    borderRadius: 8,
    padding: 8,
  },
  event: {
    fontSize: 12,
    color: "#8b949e",
    paddingVertical: 3,
    paddingHorizontal: 6,
    fontFamily: "monospace",
  },
  current: {
    color: "#fff",
    backgroundColor: "#1f6feb33",
    borderRadius: 4,
    fontWeight: "bold",
  },
  future: {
    opacity: 0.4,
  },
});
