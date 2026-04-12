import { View, Text, StyleSheet } from "react-native";

const SUIT_SYMBOLS: Record<string, string> = {
  s: "♠",
  h: "♥",
  d: "♦",
  c: "♣",
};

const SUIT_COLORS: Record<string, string> = {
  s: "#000",
  h: "#e00",
  d: "#e00",
  c: "#000",
};

export function Card({
  card,
  faceDown = false,
}: {
  card: string;
  faceDown?: boolean;
}) {
  if (faceDown || !card) {
    return (
      <View style={[styles.card, styles.faceDown]}>
        <Text style={styles.faceDownText}>🂠</Text>
      </View>
    );
  }

  const rank = card[0];
  const suit = card[1];
  const color = SUIT_COLORS[suit] || "#000";

  return (
    <View style={styles.card}>
      <Text style={[styles.rank, { color }]}>{rank}</Text>
      <Text style={[styles.suit, { color }]}>{SUIT_SYMBOLS[suit] || suit}</Text>
    </View>
  );
}

export function CardRow({ cards }: { cards: string[] }) {
  return (
    <View style={styles.row}>
      {cards.map((c, i) => (
        <Card key={i} card={c} />
      ))}
    </View>
  );
}

const styles = StyleSheet.create({
  card: {
    width: 48,
    height: 68,
    backgroundColor: "#fff",
    borderRadius: 6,
    borderWidth: 1,
    borderColor: "#ccc",
    alignItems: "center",
    justifyContent: "center",
    marginHorizontal: 3,
    shadowColor: "#000",
    shadowOffset: { width: 0, height: 1 },
    shadowOpacity: 0.15,
    shadowRadius: 2,
  },
  faceDown: {
    backgroundColor: "#2855a0",
    borderColor: "#1a3d7a",
  },
  faceDownText: {
    fontSize: 28,
  },
  rank: {
    fontSize: 20,
    fontWeight: "bold",
    lineHeight: 22,
  },
  suit: {
    fontSize: 18,
    lineHeight: 20,
  },
  row: {
    flexDirection: "row",
    alignItems: "center",
  },
});
