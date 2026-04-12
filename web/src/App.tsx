import { useState, useEffect, useCallback } from "react";
import { View, Text, Pressable, StyleSheet } from "react-native";
import { PokerTable } from "./PokerTable";
import { EventLog } from "./EventLog";
import { useSimulation } from "./useSimulation";
import { buildTableState } from "./types";

export function App() {
  const [seed, setSeed] = useState(42n);
  const [numPlayers, setNumPlayers] = useState(3);
  const [autoPlay, setAutoPlay] = useState(false);

  const sim = useSimulation({
    numPlayers,
    startingChips: 1000,
    smallBlind: 10,
    strategy: "passive",
    rngSeed: seed,
  });

  const tableState = buildTableState(sim.events, sim.currentIndex);

  // Auto-play: step forward every 300ms
  useEffect(() => {
    if (!autoPlay) return;
    if (sim.currentIndex >= sim.events.length - 1) {
      setAutoPlay(false);
      return;
    }
    const timer = setInterval(sim.stepForward, 300);
    return () => clearInterval(timer);
  }, [autoPlay, sim.currentIndex, sim.events.length, sim.stepForward]);

  const handleKeyDown = useCallback(
    (e: KeyboardEvent) => {
      if (e.key === "ArrowRight") sim.stepForward();
      else if (e.key === "ArrowLeft") sim.stepBackward();
      else if (e.key === " ") {
        e.preventDefault();
        setAutoPlay((a) => !a);
      } else if (e.key === "End") sim.jumpToEnd();
      else if (e.key === "Home") sim.jumpTo(0);
    },
    [sim]
  );

  useEffect(() => {
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [handleKeyDown]);

  if (sim.loading) {
    return (
      <View style={styles.loading} testID="loading">
        <Text style={styles.loadingText}>Loading WASM...</Text>
      </View>
    );
  }

  if (sim.error) {
    return (
      <View style={styles.loading} testID="error">
        <Text style={styles.errorText}>Error: {sim.error}</Text>
      </View>
    );
  }

  return (
    <View style={styles.container} testID="app">
      <View style={styles.header}>
        <Text style={styles.title}>Cardcore Poker</Text>
        <View style={styles.controls}>
          <Btn title="◀" onPress={sim.stepBackward} testID="btn-prev" />
          <Btn
            title={autoPlay ? "⏸" : "▶"}
            onPress={() => setAutoPlay((a) => !a)}
            testID="btn-play"
          />
          <Btn title="▶▶" onPress={sim.stepForward} testID="btn-next" />
          <Btn title="⏭" onPress={sim.jumpToEnd} testID="btn-end" />
          <Text style={styles.counter}>
            {sim.currentIndex + 1} / {sim.events.length}
          </Text>
        </View>
        <View style={styles.controls}>
          {[2, 3, 4].map((n) => (
            <Btn
              key={n}
              title={`${n}P`}
              onPress={() => setNumPlayers(n)}
              active={numPlayers === n}
              testID={`btn-${n}p`}
            />
          ))}
          <Btn
            title="New"
            onPress={() => setSeed((s) => s + 1n)}
            testID="btn-new"
          />
        </View>
      </View>

      <PokerTable state={tableState} />

      <View style={styles.logContainer}>
        <EventLog
          events={sim.events}
          currentIndex={sim.currentIndex}
          onSelect={sim.jumpTo}
        />
      </View>
    </View>
  );
}

function Btn({
  title,
  onPress,
  active,
  testID,
}: {
  title: string;
  onPress: () => void;
  active?: boolean;
  testID?: string;
}) {
  return (
    <Pressable
      onPress={onPress}
      style={[styles.btn, active && styles.btnActive]}
      testID={testID}
    >
      <Text style={[styles.btnText, active && styles.btnTextActive]}>
        {title}
      </Text>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: "#0d1117",
  },
  loading: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    backgroundColor: "#0d1117",
  },
  loadingText: {
    color: "#8b949e",
    fontSize: 18,
  },
  errorText: {
    color: "#e74c3c",
    fontSize: 16,
  },
  header: {
    padding: 16,
    alignItems: "center",
    borderBottomWidth: 1,
    borderBottomColor: "#21262d",
  },
  title: {
    color: "#fff",
    fontSize: 24,
    fontWeight: "bold",
    marginBottom: 12,
  },
  controls: {
    flexDirection: "row",
    alignItems: "center",
    marginBottom: 8,
  },
  counter: {
    color: "#8b949e",
    marginLeft: 12,
    fontSize: 13,
    fontFamily: "monospace",
  },
  btn: {
    paddingHorizontal: 12,
    paddingVertical: 6,
    backgroundColor: "#21262d",
    borderRadius: 6,
    marginHorizontal: 3,
  },
  btnActive: {
    backgroundColor: "#1f6feb",
  },
  btnText: {
    color: "#c9d1d9",
    fontSize: 13,
    fontWeight: "bold",
  },
  btnTextActive: {
    color: "#fff",
  },
  logContainer: {
    padding: 16,
  },
});
