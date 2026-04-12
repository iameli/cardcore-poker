import { useState, useEffect, useCallback } from "react";
import type { GameEvent } from "./types";

interface SimulationState {
  events: GameEvent[];
  loading: boolean;
  error: string | null;
  currentIndex: number;
}

export function useSimulation(config: {
  numPlayers: number;
  startingChips: number;
  smallBlind: number;
  strategy: string;
  rngSeed: bigint;
}) {
  const [state, setState] = useState<SimulationState>({
    events: [],
    loading: true,
    error: null,
    currentIndex: 0,
  });

  useEffect(() => {
    let cancelled = false;

    async function run() {
      try {
        const wasm = await import("../../../pkg/cardcore_poker.js");
        await wasm.default();

        const json = wasm.simulate_game(
          config.numPlayers,
          BigInt(config.startingChips),
          BigInt(config.smallBlind),
          config.strategy,
          config.rngSeed
        );
        const events: GameEvent[] = JSON.parse(json);

        if (!cancelled) {
          setState({ events, loading: false, error: null, currentIndex: 0 });
        }
      } catch (e: any) {
        if (!cancelled) {
          setState({
            events: [],
            loading: false,
            error: e.toString(),
            currentIndex: 0,
          });
        }
      }
    }

    run();
    return () => {
      cancelled = true;
    };
  }, [
    config.numPlayers,
    config.startingChips,
    config.smallBlind,
    config.strategy,
    config.rngSeed,
  ]);

  const stepForward = useCallback(() => {
    setState((s) => ({
      ...s,
      currentIndex: Math.min(s.currentIndex + 1, s.events.length - 1),
    }));
  }, []);

  const stepBackward = useCallback(() => {
    setState((s) => ({
      ...s,
      currentIndex: Math.max(s.currentIndex - 1, 0),
    }));
  }, []);

  const jumpTo = useCallback((index: number) => {
    setState((s) => ({
      ...s,
      currentIndex: Math.max(0, Math.min(index, s.events.length - 1)),
    }));
  }, []);

  const jumpToEnd = useCallback(() => {
    setState((s) => ({
      ...s,
      currentIndex: s.events.length - 1,
    }));
  }, []);

  return {
    ...state,
    stepForward,
    stepBackward,
    jumpTo,
    jumpToEnd,
  };
}
