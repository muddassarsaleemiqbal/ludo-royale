import { useSyncExternalStore } from "react";
import { Realtime } from "ably";
import type { Message } from "ably";
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
  player: number | null; connected: boolean; lobbies: LobbySummary[];
  lobby: Lobby | null; error: string | null;
};

const api = (import.meta.env.VITE_API_URL as string | undefined)?.replace(/\/$/, "") ?? "http://localhost:8080";

class OnlineStore {
  private listeners = new Set<() => void>();
  private socket: WebSocket | null = null;
  private ably: Realtime | null = null;
  private token = localStorage.getItem("ludo-online-token");
  private snapshot: Snapshot = {
    user: null, model: null, lobbyId: null, player: null, connected: false,
    lobbies: [], lobby: null, error: null
  };

  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => this.listeners.delete(listener); };
  getSnapshot = () => this.snapshot;

  async restore() {
    if (!this.token || this.snapshot.user) return;
    try {
      const response = await fetch(`${api}/api/me`, { headers: { Authorization: `Bearer ${this.token}` } });
      if (!response.ok) throw new Error("Your session expired");
      this.set({ user: await response.json(), error: null });
      this.connect();
    } catch (error) { this.logout(); this.set({ error: String(error) }); }
  }

  async authenticate(kind: "login" | "register", email: string, password: string, displayName: string) {
    const response = await fetch(`${api}/api/auth/${kind}`, {
      method: "POST", headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ email, password, display_name: displayName || undefined })
    });
    const body = await response.json();
    if (!response.ok) throw new Error(body.error ?? "Authentication failed");
    this.token = body.token;
    localStorage.setItem("ludo-online-token", body.token);
    this.set({ user: body.user, error: null });
    this.connect();
  }

  listLobbies() { this.send({ type: "list_lobbies" }); }
  createLobby(options: { name: string; rule_preset: string; bot_difficulty: string; is_public: boolean }) {
    this.send({ type: "create_lobby", ...options });
  }
  requestJoin(lobbyId: string) { this.send({ type: "request_join", lobby_id: lobbyId }); }
  respondJoin(requestId: string, accept: boolean) { this.send({ type: "respond_join", request_id: requestId, accept }); }
  leaveLobby() { if (this.snapshot.lobby) this.send({ type: "leave_lobby", lobby_id: this.snapshot.lobby.id }); this.set({ lobby: null }); }
  startGame() { if (this.snapshot.lobby) this.send({ type: "start_game", lobby_id: this.snapshot.lobby.id }); }
  showLocal() { this.set({ model: null, lobbyId: null, player: null, lobby: null }); }
  roll() {
    if (this.snapshot.lobbyId && this.snapshot.model)
      this.send({ type: "roll", lobby_id: this.snapshot.lobbyId, revision: this.snapshot.model.revision });
  }
  move(token: number) {
    if (this.snapshot.lobbyId && this.snapshot.model)
      this.send({ type: "move", lobby_id: this.snapshot.lobbyId, revision: this.snapshot.model.revision, token });
  }
  logout() {
    this.socket?.close(); this.ably?.close(); this.ably = null;
    this.token = null; localStorage.removeItem("ludo-online-token");
    this.snapshot = { user: null, model: null, lobbyId: null, player: null, connected: false, lobbies: [], lobby: null, error: null };
    this.emit();
  }

  private connect() {
    if (!this.token || this.socket?.readyState === WebSocket.OPEN) return;
    const url = new URL(api); url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
    url.pathname = "/api/online"; url.searchParams.set("token", this.token);
    this.socket = new WebSocket(url);
    this.socket.onopen = () => {
      this.set({ connected: true, error: null });
      this.connectAbly();
      if (this.snapshot.lobbyId) this.send({ type: "sync", lobby_id: this.snapshot.lobbyId });
    };
    this.socket.onclose = () => {
      this.socket = null; this.set({ connected: false });
      if (this.token) window.setTimeout(() => this.connect(), 1500);
    };
    this.socket.onmessage = (event) => this.handleMessage(JSON.parse(event.data));
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
          if (!response.ok) throw new Error("Ably authentication is unavailable");
          callback(null, await response.text());
        } catch (error) {
          callback(String(error), null);
        }
      }
    });
    const userChannel = this.ably.channels.get(`ludo:user:${this.snapshot.user.id}`);
    const lobbyChannel = this.ably.channels.get("ludo:lobbies");
    void Promise.all([
      userChannel.subscribe("event", (message: Message) => this.handleMessage(message.data)),
      lobbyChannel.subscribe("changed", () => this.listLobbies())
    ]).then(() => this.listLobbies());
  }
  private handleMessage(message: any) {
    if (!message || typeof message.type !== "string") return;
    if (message.type === "lobby_list") this.set({ lobbies: message.lobbies });
    if (message.type === "lobby") this.set({ lobby: message.lobby });
    if (message.type === "join_requested") this.set({ error: null });
    if (message.type === "game_started")
      this.set({ lobby: null, lobbyId: message.lobby_id, player: message.player, model: message.model });
    if (message.type === "state") this.set({ model: message.model });
    if (message.type === "error") this.set({ error: message.message });
  }
  private send(value: unknown) {
    if (this.socket?.readyState !== WebSocket.OPEN) { this.set({ error: "Online server is not connected" }); return; }
    this.socket.send(JSON.stringify(value));
  }
  private set(change: Partial<Snapshot>) { this.snapshot = { ...this.snapshot, ...change }; this.emit(); }
  private emit() { for (const listener of this.listeners) listener(); }
}

export const onlineStore = new OnlineStore();
export function useOnlineStore() { return useSyncExternalStore(onlineStore.subscribe, onlineStore.getSnapshot); }
