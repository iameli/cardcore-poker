import { View, Text, StyleSheet } from "react-native";
import { CardRow } from "./Card";
import { PlayerSeat } from "./PlayerSeat";
import type { TableState } from "./types";

const STREET_LABELS: Record<string, string> = {
  preflop: "Pre-Flop",
  flop: "Flop",
  turn: "Turn",
  river: "River",
};

export function PokerTable({ state }: { state: TableState }) {
  return (
    <View style={styles.table} testID="poker-table">
      <View style={styles.felt}>
        <Text style={styles.street}>
          {STREET_LABELS[state.street] || state.street}
        </Text>

        {state.communityCards.length > 0 && (
          <View style={styles.community} testID="community-cards">
            <CardRow cards={state.communityCards} />
          </View>
        )}

        {state.pot > 0 && (
          <Text style={styles.pot} testID="pot">
            Pot: {state.pot}
          </Text>
        )}
      </View>

      <View style={styles.seats}>
        {state.players.map((player, i) => (
          <PlayerSeat
            key={i}
            player={player}
            index={i}
            isShowdown={state.showdown}
          />
        ))}
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  table: {
    flex: 1,
    backgroundColor: "#0d1117",
    alignItems: "center",
    justifyContent: "center",
    padding: 20,
  },
  felt: {
    backgroundColor: "#1b5e20",
    borderRadius: 120,
    paddingVertical: 40,
    paddingHorizontal: 60,
    alignItems: "center",
    justifyContent: "center",
    minHeight: 180,
    minWidth: 300,
    borderWidth: 4,
    borderColor: "#2e7d32",
    shadowColor: "#000",
    shadowOffset: { width: 0, height: 4 },
    shadowOpacity: 0.3,
    shadowRadius: 8,
  },
  street: {
    fontSize: 16,
    color: "#a5d6a7",
    fontWeight: "bold",
    marginBottom: 12,
  },
  community: {
    marginBottom: 12,
  },
  pot: {
    fontSize: 14,
    color: "#fff",
    backgroundColor: "rgba(0,0,0,0.3)",
    paddingHorizontal: 12,
    paddingVertical: 4,
    borderRadius: 8,
  },
  seats: {
    flexDirection: "row",
    flexWrap: "wrap",
    justifyContent: "center",
    marginTop: 20,
  },
});
