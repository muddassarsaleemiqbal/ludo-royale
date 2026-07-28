import { Realtime } from "ably";
import type { Message } from "ably";
import { useSyncExternalStore } from "react";
import type { GameViewModel } from "./types";

export type User = { id: string; email: string; display_name: string };
export type LobbySummary = {
  id: string; name: string; host_name: string; human_players: number;
  rule_preset: string; bot_difficulty: string; status: string;
  is_host: boolean; requested: boolean;
};
export type Lobby = {
  id: string; name: string; host_user_id: string; rule_preset: string;
  bot_difficulty: string; status: string;
  seats: { seat: number; user_id: string | null; name: string; is_bot: boolean }[];
  requests: { id: string; user_id: string; display_name: string }[];
};
type Snapshot = {
  user: User | null; model: GameViewModel | null; lobbyId: string | null;
  player: number | null; connected: boolean; realtimeConnected: boolean;
  lobbies: LobbySummary[]; lobby: Lobby | null; error: string | null;
  pending: string | null;
};
type ServerMessage =
  | { type: "ready"; user: User }
  | { type: "lobby_list"; lobbies: LobbySummary[] }
  | { type: "lobby"; lobby: Lobby }
  | { type: "join_requested"; lobby_id: string }
  | { type: "game_started"; lobby_id: string; player: number; model: GameViewModel }
  | { type: "state"; lobby_id: string; model: GameViewModel }
  | { type: "ack"; command_id: string }
  | {
      type: "error"; command_id: string | null; code: string;
      message: string; recoverable: boolean
    };

const api = (import.meta.env.VITE_API_URL as string | undefined)?.replace(/\/$/, "")
  ?? "http://localhost:8080";
const tokenKey = "ludo-online-token";
const lobbyKey = "ludo-online-lobby";

class OnlineStore {
  private listeners = new Set<() => void>();
  private socket: WebSocket | null = null;
  private ably: Realtime | null = null;
  private token = localStorage.getItem(tokenKey);
  private reconnectTimer: number | null = null;
  private reconnectAttempt = 0;
  private commands = new Map<string, string | null>();
  private snapshot: Snapshot = {
    user: null, model: null, lobbyId: localStorage.getItem(lobbyKey), player: null,
    connected: false, realtimeConnected: false, lobbies: [], lobby: null,
    error: null, pending: null
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
      if (!response.ok) throw new Error("Your session expired. Please sign in again.");
      this.set({ user: await response.json() as User, error: null });
      this.connect();
    } catch (error) {
      this.resetSession();
      this.set({ error: messageFrom(error) });
    }
  }

  async authenticate(
    kind: "login" | "register",
    email: string,
    password: string,
    displayName: string
  ) {
    this.set({ pending: "auth", error: null });
    try {
      const response = await fetch(`${api}/api/auth/${kind}`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          email: email.trim(),
          password,
          display_name: displayName.trim() || undefined
        })
      });
      const body = await response.json() as { token?: string; user?: User; error?: string };
      if (!response.ok || !body.token || !body.user)
        throw new Error(body.error ?? "Authentication failed");
      this.token = body.token;
      localStorage.setItem(tokenKey, body.token);
      this.set({ user: body.user, error: null, pending: null });
      this.connect();
    } catch (error) {
      this.set({ pending: null, error: messageFrom(error) });
      throw error;
    }
  }

  clearError() { this.set({ error: null }); }
  listLobbies() { this.send({ type: "list_lobbies" }); }
  createLobby(options: { name: string; rule_preset: string; bot_difficulty: string }) {
    this.command("create", { type: "create_lobby", ...options, is_public: true });
  }
  requestJoin(lobbyId: string) {
    this.command(`join:${lobbyId}`, { type: "request_join", lobby_id: lobbyId });
  }
  respondJoin(requestId: string, accept: boolean) {
    this.command(`request:${requestId}`, {
      type: "respond_join", request_id: requestId, accept
    });
  }
  leaveLobby() {
    if (!this.snapshot.lobby) return;
    this.send({ type: "leave_lobby", lobby_id: this.snapshot.lobby.id });
    this.set({ lobby: null, pending: null, error: null });
  }
  startGame() {
    if (this.snapshot.lobby)
      this.command("start", { type: "start_game", lobby_id: this.snapshot.lobby.id });
  }
  showLocal() {
    localStorage.removeItem(lobbyKey);
    this.set({ model: null, lobbyId: null, player: null, lobby: null, error: null });
  }
  roll() {
    if (this.snapshot.lobbyId && this.snapshot.model)
      this.send({
        type: "roll", lobby_id: this.snapshot.lobbyId,
        revision: this.snapshot.model.revision
      });
  }
  move(token: number) {
    if (this.snapshot.lobbyId && this.snapshot.model)
      this.send({
        type: "move", lobby_id: this.snapshot.lobbyId,
        revision: this.snapshot.model.revision, token
      });
  }
  logout() {
    const token = this.token;
    if (token) {
      void fetch(`${api}/api/auth/logout`, {
        method: "POST", headers: { Authorization: `Bearer ${token}` }
      }).catch(() => undefined);
    }
    this.resetSession();
  }

  private connect() {
    if (!this.token || this.socket?.readyState === WebSocket.OPEN
      || this.socket?.readyState === WebSocket.CONNECTING) return;
    if (this.reconnectTimer !== null) {
      window.clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    const url = new URL(api);
    url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
    url.pathname = "/api/online";
    this.socket = new WebSocket(url, ["ludo", this.token]);
    this.socket.onopen = () => {
      this.reconnectAttempt = 0;
      this.set({ connected: true, error: null });
      this.connectAbly();
      if (this.snapshot.lobbyId)
        this.send({ type: "sync", lobby_id: this.snapshot.lobbyId });
    };
    this.socket.onclose = () => {
      this.socket = null;
      this.set({ connected: false, pending: null });
      if (!this.token) return;
      const delay = Math.min(15_000, 1_000 * 2 ** this.reconnectAttempt);
      this.reconnectAttempt = Math.min(this.reconnectAttempt + 1, 4);
      this.reconnectTimer = window.setTimeout(() => this.connect(), delay);
    };
    this.socket.onerror = () => this.set({
      error: "The game server connection was interrupted. Reconnecting…"
    });
    this.socket.onmessage = (event) => {
      try {
        this.handleMessage(JSON.parse(String(event.data)) as unknown);
      } catch {
        this.set({ error: "The server sent an unreadable update. Reconnecting…" });
        this.socket?.close();
      }
    };
  }

  private connectAbly() {
    if (!this.token || !this.snapshot.user || this.ably) return;
    const authToken = this.token;
    this.ably = new Realtime({
      clientId: this.snapshot.user.id,
      authCallback: async (_params, callback) => {
        try {
          const response = await fetch(`${api}/api/ably/token`, {
            headers: { Authorization: `Bearer ${authToken}` }
          });
          if (!response.ok) throw new Error("Realtime authentication is unavailable");
          callback(null, await response.text());
        } catch (error) {
          callback(messageFrom(error), null);
        }
      }
    });
    this.ably.connection.on("connected", () => this.set({ realtimeConnected: true }));
    this.ably.connection.on("disconnected", () => this.set({ realtimeConnected: false }));
    this.ably.connection.on("failed", () => this.set({
      realtimeConnected: false,
      error: "Realtime updates are unavailable. Your game server is reconnecting."
    }));
    const userChannel = this.ably.channels.get(`ludo:user:${this.snapshot.user.id}`);
    const lobbyChannel = this.ably.channels.get("ludo:lobbies");
    void Promise.all([
      userChannel.subscribe("event", (message: Message) => this.handleMessage(message.data)),
      lobbyChannel.subscribe("changed", () => this.listLobbies())
    ]).then(() => this.listLobbies()).catch((error: unknown) => {
      this.set({ error: `Could not subscribe to realtime updates: ${messageFrom(error)}` });
    });
  }

  private handleMessage(value: unknown) {
    if (!isServerMessage(value)) return;
    if (value.type === "ready") return;
    if (value.type === "ack") {
      this.commands.delete(value.command_id);
      if (this.commands.size === 0) this.set({ pending: null });
      return;
    }
    if (value.type === "lobby_list") {
      this.set({ lobbies: value.lobbies, pending: null });
      return;
    }
    if (value.type === "lobby") {
      this.set({ lobby: value.lobby, pending: null, error: null });
      return;
    }
    if (value.type === "join_requested") {
      this.set({ pending: null, error: null });
      return;
    }
    if (value.type === "game_started") {
      localStorage.setItem(lobbyKey, value.lobby_id);
      this.set({
        lobby: null, lobbyId: value.lobby_id, player: value.player,
        model: value.model, pending: null, error: null
      });
      return;
    }
    if (value.type === "state") {
      if (this.snapshot.lobbyId && value.lobby_id !== this.snapshot.lobbyId) return;
      if (this.snapshot.model && value.model.revision < this.snapshot.model.revision) return;
      this.set({ model: value.model, pending: null, error: null });
      return;
    }
    if (value.command_id) this.commands.delete(value.command_id);
    this.set({ error: value.message, pending: this.commands.size === 0 ? null : this.snapshot.pending });
    if (value.code === "stale_revision" && this.snapshot.lobbyId)
      this.send({ type: "sync", lobby_id: this.snapshot.lobbyId });
    if (value.code === "game_not_found") {
      localStorage.removeItem(lobbyKey);
      this.set({ lobbyId: null, model: null, player: null });
    }
  }

  private command(pending: string, value: unknown) {
    this.set({ pending, error: null });
    if (!this.send(value, pending)) this.set({ pending: null });
  }
  private send(value: unknown, pending: string | null = null) {
    if (this.socket?.readyState !== WebSocket.OPEN) {
      this.set({ error: "The online server is reconnecting. Please try again shortly." });
      return false;
    }
    const commandId = crypto.randomUUID();
    this.commands.set(commandId, pending);
    this.socket.send(JSON.stringify({ command_id: commandId, ...value as object }));
    return true;
  }
  private resetSession() {
    if (this.reconnectTimer !== null) window.clearTimeout(this.reconnectTimer);
    this.reconnectTimer = null;
    this.socket?.close();
    this.socket = null;
    this.ably?.close();
    this.ably = null;
    this.commands.clear();
    this.token = null;
    localStorage.removeItem(tokenKey);
    localStorage.removeItem(lobbyKey);
    this.snapshot = {
      user: null, model: null, lobbyId: null, player: null, connected: false,
      realtimeConnected: false, lobbies: [], lobby: null, error: null, pending: null
    };
    this.emit();
  }
  private set(change: Partial<Snapshot>) {
    this.snapshot = { ...this.snapshot, ...change };
    this.emit();
  }
  private emit() { for (const listener of this.listeners) listener(); }
}

function isServerMessage(value: unknown): value is ServerMessage {
  return typeof value === "object" && value !== null
    && "type" in value && typeof value.type === "string";
}
function messageFrom(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

export const onlineStore = new OnlineStore();
export function useOnlineStore() {
  return useSyncExternalStore(onlineStore.subscribe, onlineStore.getSnapshot);
}
