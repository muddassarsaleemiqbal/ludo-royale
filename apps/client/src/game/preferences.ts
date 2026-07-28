import { useSyncExternalStore } from "react";

export type GamePreferences = {
  sound: boolean;
  motion: boolean;
  safeHints: boolean;
};

const storageKey = "ludo-preferences";
const defaults: GamePreferences = {
  sound: true,
  motion: !matchMedia("(prefers-reduced-motion: reduce)").matches,
  safeHints: true
};

function load(): GamePreferences {
  try {
    return { ...defaults, ...JSON.parse(localStorage.getItem(storageKey) ?? "{}") };
  } catch {
    return defaults;
  }
}

class PreferenceStore {
  private value = load();
  private listeners = new Set<() => void>();

  constructor() {
    document.documentElement.dataset.motion = this.value.motion ? "full" : "reduced";
  }

  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };
  getSnapshot = () => this.value;
  set<K extends keyof GamePreferences>(key: K, value: GamePreferences[K]) {
    this.value = { ...this.value, [key]: value };
    localStorage.setItem(storageKey, JSON.stringify(this.value));
    document.documentElement.dataset.motion = this.value.motion ? "full" : "reduced";
    for (const listener of this.listeners) listener();
  }
}

export const preferenceStore = new PreferenceStore();
export function usePreferences() {
  return useSyncExternalStore(preferenceStore.subscribe, preferenceStore.getSnapshot);
}
