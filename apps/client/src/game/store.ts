import { useSyncExternalStore } from "react";
import { createGameClient } from "./clients";
import type {
  GameAction,
  GameClient,
  GameViewModel,
  RuntimeEffect
} from "./types";

type StoreSnapshot = {
  model: GameViewModel | null;
  starting: boolean;
  fatalError: string | null;
  platform: "web" | "native";
  resumeAvailable: boolean;
};

const localGameKey = "ludo-local-game-v1";

class GameStore {
  private readonly client: GameClient = createGameClient();
  private initialization: Promise<void> | null = null;
  private listeners = new Set<() => void>();
  private snapshot: StoreSnapshot = {
    model: null,
    starting: true,
    fatalError: null,
    platform: window.__TAURI__ ? "native" : "web",
    resumeAvailable: false
  };

  readonly subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  readonly getSnapshot = () => this.snapshot;

  initialize() {
    this.initialization ??= this.initializeOnce();
    return this.initialization;
  }

  private async initializeOnce() {
    try {
      const model = await this.client.initialize();
      const saved = localStorage.getItem(localGameKey);
      if (saved) {
        try {
          await this.client.restore(saved);
          const resumed = await this.client.dispatch("Resume");
          this.setSnapshot({
            ...this.snapshot,
            model: resumed.model,
            starting: false,
            resumeAvailable: resumed.model.revision > 0 && resumed.model.winner === null
          });
          for (const effect of resumed.effects) void this.execute(effect);
          return;
        } catch {
          localStorage.removeItem(localGameKey);
        }
      }
      this.setSnapshot({ ...this.snapshot, model, starting: false, resumeAvailable: false });
    } catch (error) {
      this.setSnapshot({
        ...this.snapshot,
        starting: false,
        fatalError: String(error)
      });
    }
  }

  async dispatch(action: GameAction) {
    try {
      const update = await this.client.dispatch(action);
      const isNewGame = action === "NewGame"
        || (typeof action === "object" && "NewGameWith" in action);
      if (
        update.model.winner === null &&
        update.model.revision > 0 &&
        !isNewGame
      ) {
        try {
          localStorage.setItem(localGameKey, await this.client.serialize());
        } catch {
          // Persistence is best-effort; storage restrictions cannot stop play.
        }
      } else if (update.model.winner !== null || isNewGame) {
        localStorage.removeItem(localGameKey);
      }
      this.setSnapshot({
        ...this.snapshot,
        model: update.model,
        resumeAvailable: update.model.winner === null && update.model.revision > 0
      });
      for (const effect of update.effects) void this.execute(effect);
    } catch (error) {
      this.setSnapshot({ ...this.snapshot, fatalError: String(error) });
    }
  }

  private async execute(effect: RuntimeEffect) {
    if ("DelayBot" in effect) {
      await new Promise((resolve) =>
        setTimeout(resolve, effect.DelayBot.milliseconds)
      );
      await this.dispatch({ ContinueBot: effect.DelayBot.effect });
      return;
    }
    if ("GenerateDice" in effect) {
      const value = await this.client.randomDice();
      await this.dispatch({
        DiceReady: { effect: effect.GenerateDice.effect, value }
      });
      return;
    }
    const decision = await this.client.evaluateBot(effect.EvaluateBot.request);
    await this.dispatch({
      BotReady: { effect: effect.EvaluateBot.effect, decision }
    });
  }

  private setSnapshot(snapshot: StoreSnapshot) {
    this.snapshot = snapshot;
    for (const listener of this.listeners) listener();
  }
}

export const gameStore = new GameStore();

export function useGameStore() {
  return useSyncExternalStore(gameStore.subscribe, gameStore.getSnapshot);
}
