/// <reference types="vite/client" />

declare module "*ludo_web.js" {
  export default function init(): Promise<void>;
  export class WasmGame {
    constructor();
    snapshot_json(): string;
    state_json(): string;
    restore_json(state: string): string;
    dispatch_json(action: string): string;
  }
  export function evaluate_bot_json(request: string): string;
}

interface Window {
  __TAURI__?: {
    core: {
      invoke<T>(command: string, args?: Record<string, unknown>): Promise<T>;
    };
  };
}
