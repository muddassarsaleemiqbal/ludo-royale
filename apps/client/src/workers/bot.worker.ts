import init, { evaluate_bot_json } from "../wasm/pkg/ludo_web.js";
import type { BotDecision } from "../game/types";

const ready = init();

self.onmessage = async (
  event: MessageEvent<{ id: number; request: unknown }>
) => {
  try {
    await ready;
    const decision = JSON.parse(
      evaluate_bot_json(JSON.stringify(event.data.request))
    ) as BotDecision;
    self.postMessage({ id: event.data.id, decision });
  } catch (error) {
    self.postMessage({ id: event.data.id, error: String(error) });
  }
};
