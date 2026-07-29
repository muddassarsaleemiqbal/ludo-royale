import { beforeEach, describe, expect, it, vi } from "vitest";
import { createGameClient, TauriGameClient, WasmGameClient } from "./clients";

describe("game client selection", () => {
  beforeEach(() => {
    delete window.__TAURI__;
  });

  it("selects WebAssembly in browsers", () => {
    expect(createGameClient()).toBeInstanceOf(WasmGameClient);
  });

  it("selects the native bridge in Tauri", () => {
    window.__TAURI__ = { core: { invoke: vi.fn() } };
    expect(createGameClient()).toBeInstanceOf(TauriGameClient);
  });
});

describe("TauriGameClient", () => {
  it("routes every operation through the expected native command", async () => {
    const invoke = vi.fn().mockResolvedValue({});
    window.__TAURI__ = { core: { invoke } };
    const client = new TauriGameClient();

    await client.initialize();
    await client.dispatch("Roll");
    await client.evaluateBot({ player: 2 });
    await client.randomDice();
    await client.serialize();
    await client.restore("snapshot");

    expect(invoke).toHaveBeenNthCalledWith(1, "snapshot");
    expect(invoke).toHaveBeenNthCalledWith(2, "dispatch", { action: "Roll" });
    expect(invoke).toHaveBeenNthCalledWith(3, "evaluate_bot", { request: { player: 2 } });
    expect(invoke).toHaveBeenNthCalledWith(4, "random_dice");
    expect(invoke).toHaveBeenNthCalledWith(5, "state_json");
    expect(invoke).toHaveBeenNthCalledWith(6, "restore_state", { snapshot: "snapshot" });
  });

  it("fails clearly when the native bridge is unavailable", () => {
    delete window.__TAURI__;
    const client = new TauriGameClient();
    expect(() => client.initialize()).toThrow("Tauri bridge is unavailable");
  });
});

describe("WasmGameClient guards", () => {
  it("rejects operations before initialization", async () => {
    const client = new WasmGameClient();
    await expect(client.dispatch("Roll")).rejects.toThrow("WASM game has not initialized");
    await expect(client.serialize()).rejects.toThrow("WASM game has not initialized");
    await expect(client.restore("{}")).rejects.toThrow("WASM game has not initialized");
    await expect(client.evaluateBot({})).rejects.toThrow("AI worker unavailable");
  });

  it("generates only valid dice values", async () => {
    const client = new WasmGameClient();
    const values = await Promise.all(Array.from({ length: 100 }, () => client.randomDice()));
    expect(values.every(value => value >= 1 && value <= 6)).toBe(true);
    expect(new Set(values).size).toBeGreaterThan(1);
  });
});
