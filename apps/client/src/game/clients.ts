import type {
  BotDecision,
  GameAction,
  GameClient,
  GameViewModel,
  RuntimeUpdate
} from "./types";

type WasmModule = typeof import("../wasm/pkg/ludo_web.js");

export class WasmGameClient implements GameClient {
  private module?: WasmModule;
  private game?: InstanceType<WasmModule["WasmGame"]>;
  private worker?: Worker;
  private nextWorkerId = 1;
  private workerRequests = new Map<
    number,
    { resolve: (value: BotDecision) => void; reject: (reason: unknown) => void }
  >();

  async initialize(): Promise<GameViewModel> {
    this.module = await import("../wasm/pkg/ludo_web.js");
    await this.module.default();
    this.game = new this.module.WasmGame();
    this.worker = new Worker(
      new URL("../workers/bot.worker.ts", import.meta.url),
      { type: "module" }
    );
    this.worker.onmessage = (
      event: MessageEvent<
        | { id: number; decision: BotDecision }
        | { id: number; error: string }
      >
    ) => {
      const pending = this.workerRequests.get(event.data.id);
      if (!pending) return;
      this.workerRequests.delete(event.data.id);
      if ("error" in event.data) pending.reject(new Error(event.data.error));
      else pending.resolve(event.data.decision);
    };
    return JSON.parse(this.game.snapshot_json()) as GameViewModel;
  }

  async dispatch(action: GameAction): Promise<RuntimeUpdate> {
    if (!this.game) throw new Error("WASM game has not initialized");
    return JSON.parse(
      this.game.dispatch_json(JSON.stringify(action))
    ) as RuntimeUpdate;
  }

  evaluateBot(request: unknown): Promise<BotDecision> {
    if (!this.worker) return Promise.reject(new Error("AI worker unavailable"));
    const id = this.nextWorkerId++;
    return new Promise((resolve, reject) => {
      this.workerRequests.set(id, { resolve, reject });
      this.worker?.postMessage({ id, request });
    });
  }

  async randomDice(): Promise<number> {
    const values = new Uint32Array(1);
    const range = 0x1_0000_0000;
    const limit = range - (range % 6);
    do crypto.getRandomValues(values);
    while ((values[0] ?? limit) >= limit);
    return ((values[0] ?? 0) % 6) + 1;
  }

  close() {
    this.worker?.terminate();
    this.workerRequests.clear();
  }
}

export class TauriGameClient implements GameClient {
  private get invoke() {
    const invoke = window.__TAURI__?.core.invoke;
    if (!invoke) throw new Error("Tauri bridge is unavailable");
    return invoke;
  }

  initialize() {
    return this.invoke<GameViewModel>("snapshot");
  }

  dispatch(action: GameAction) {
    return this.invoke<RuntimeUpdate>("dispatch", { action });
  }

  evaluateBot(request: unknown) {
    return this.invoke<BotDecision>("evaluate_bot", { request });
  }

  randomDice() {
    return this.invoke<number>("random_dice");
  }

  close() {}
}

export function createGameClient(): GameClient {
  return window.__TAURI__
    ? new TauriGameClient()
    : new WasmGameClient();
}
