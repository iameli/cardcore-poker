import { View, Text, StyleSheet } from "react-native";
import { CardRow, Card } from "./Card";
import type { PlayerState } from "./types";

const SEAT_COLORS = [
  "#3498db",
  "#e74c3c",
  "#2ecc71",
  "#f39c12",
  "#9b59b6",
  "#1abc9c",
];

export function PlayerSeat({
  player,
  index,
  isShowdown,
}: {
  player: PlayerState;
  index: number;
  isShowdown: boolean;
}) {
  const color = SEAT_COLORS[index % SEAT_COLORS.length];
  const name = player.did.replace("did:plc:", "");

  return (
    <View
      style={[
        styles.seat,
        player.folded && styles.folded,
        { borderColor: color },
      ]}
      testID={`player-seat-${index}`}
    >
      <Text style={[styles.name, { color }]}>{name}</Text>

      <View style={styles.cards}>
        {player.holeCards.length > 0 ? (
          <CardRow cards={player.holeCards} />
        ) : (
          <View style={styles.cards}>
            <Card card="" faceDown />
            <Card card="" faceDown />
          </View>
        )}
      </View>

      {player.lastAction && (
        <Text
          style={styles.action}
          testID={`player-action-${index}`}
          numberOfLines={2}
        >
          {player.lastAction}
        </Text>
      )}

      {player.folded && <Text style={styles.foldBadge}>FOLDED</Text>}
    </View>
  );
}

const styles = StyleSheet.create({
  seat: {
    alignItems: "center",
    padding: 12,
    borderRadius: 12,
    borderWidth: 2,
    backgroundColor: "#1a1a2e",
    minWidth: 130,
    margin: 8,
  },
  folded: {
    opacity: 0.5,
  },
  name: {
    fontSize: 14,
    fontWeight: "bold",
    marginBottom: 6,
  },
  cards: {
    flexDirection: "row",
    marginBottom: 6,
  },
  action: {
    fontSize: 11,
    color: "#aaa",
    textAlign: "center",
  },
  foldBadge: {
    fontSize: 10,
    color: "#e74c3c",
    fontWeight: "bold",
    marginTop: 4,
  },
});
