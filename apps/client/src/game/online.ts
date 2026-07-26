import { useSyncExternalStore } from "react";
import type { GameViewModel } from "./types";

type User = { id: string; email: string; display_name: string };
type Snapshot = {
  user: User | null;
  model: GameViewModel | null;
  matchId: string | null;
  player: number | null;
  connected: boolean;
  queued: boolean;
  error: string | null;
};

const api = (import.meta.env.VITE_API_URL as string | undefined)?.replace(/\/$/, "")
  ?? "http://localhost:8080";

class OnlineStore {
  private listeners = new Set<() => void>();
  private socket: WebSocket | null = null;
  private token = localStorage.getItem("ludo-online-token");
  private snapshot: Snapshot = {
    user: null, model: null, matchId: null, player: null,
    connected: false, queued: false, error: null
  };

  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };
  getSnapshot = () => this.snapshot;

  async restore() {
    if (!this.token || this.snapshot.user) return;
    try {
      const response = await fetch(`${api}/api/me`, {
        headers: { Authorization: `Bearer ${this.token}` }
      });
      if (!response.ok) throw new Error("Your session expired");
      this.set({ user: await response.json(), error: null });
      this.connect();
    } catch (error) {
      this.logout();
      this.set({ error: String(error) });
    }
  }

  async authenticate(kind: "login" | "register", email: string, password: string, displayName: string) {
    const response = await fetch(`${api}/api/auth/${kind}`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ email, password, display_name: displayName || undefined })
    });
    const body = await response.json();
    if (!response.ok) throw new Error(body.error ?? "Authentication failed");
    this.token = body.token;
    localStorage.setItem("ludo-online-token", body.token);
    this.set({ user: body.user, error: null });
    this.connect();
  }

  findMatch() { this.send({ type: "find_match" }); }
  leaveQueue() { this.send({ type: "leave_queue" }); }
  showLocal() { this.set({ model: null, matchId: null, player: null }); }
  roll() {
    if (this.snapshot.matchId && this.snapshot.model)
      this.send({ type: "roll", match_id: this.snapshot.matchId, revision: this.snapshot.model.revision });
  }
  move(token: number) {
    if (this.snapshot.matchId && this.snapshot.model)
      this.send({ type: "move", match_id: this.snapshot.matchId, revision: this.snapshot.model.revision, token });
  }
  logout() {
    this.socket?.close();
    this.token = null;
    localStorage.removeItem("ludo-online-token");
    this.snapshot = { user: null, model: null, matchId: null, player: null, connected: false, queued: false, error: null };
    this.emit();
  }

  private connect() {
    if (!this.token || this.socket?.readyState === WebSocket.OPEN) return;
    const url = new URL(api);
    url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
    url.pathname = "/api/online";
    url.searchParams.set("token", this.token);
    this.socket = new WebSocket(url);
    this.socket.onopen = () => {
      this.set({ connected: true });
      if (this.snapshot.matchId)
        this.send({ type: "sync", match_id: this.snapshot.matchId });
    };
    this.socket.onclose = () => {
      this.socket = null;
      this.set({ connected: false, queued: false });
      if (this.token) window.setTimeout(() => this.connect(), 1500);
    };
    this.socket.onmessage = (event) => {
      const message = JSON.parse(event.data);
      if (message.type === "queued") this.set({ queued: true });
      if (message.type === "queue_left") this.set({ queued: false });
      if (message.type === "match_found")
        this.set({ queued: false, matchId: message.match_id, player: message.player, model: message.model });
      if (message.type === "state") this.set({ model: message.model });
      if (message.type === "error") this.set({ error: message.message });
    };
  }
  private send(value: unknown) {
    if (this.socket?.readyState !== WebSocket.OPEN) {
      this.set({ error: "Online server is not connected" });
      return;
    }
    this.socket.send(JSON.stringify(value));
  }
  private set(change: Partial<Snapshot>) { this.snapshot = { ...this.snapshot, ...change }; this.emit(); }
  private emit() { for (const listener of this.listeners) listener(); }
}

export const onlineStore = new OnlineStore();
export function useOnlineStore() {
  return useSyncExternalStore(onlineStore.subscribe, onlineStore.getSnapshot);
}
