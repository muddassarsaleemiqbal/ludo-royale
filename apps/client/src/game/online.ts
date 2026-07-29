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
  invite_code: string; is_public: boolean; turn_seconds: number; spectator_count: number;
  ranked: boolean; rematch_mode: "vote" | "host" | "automatic";
  seats: {
    seat: number; user_id: string | null; name: string; is_bot: boolean;
    ready: boolean; presence: "online" | "reconnecting" | "offline" | "bot";
  }[];
  requests: { id: string; user_id: string; display_name: string }[];
};
export type Activity = { id: number; kind: string; message: string; created_at: string };
export type SocialPlayer = {
  user_id: string; display_name: string; level: number; rating: number;
  relationship: "friend" | "incoming" | "outgoing" | "none";
  presence: "online" | "offline";
};
export type PlayerHub = {
  profile: {
    user_id: string; display_name: string; xp: number; level: number; matches: number;
    wins: number; current_streak: number; best_streak: number; rating: number;
    selected_dice: string; selected_tokens: string;
  };
  friends: SocialPlayer[];
  matches: {
    id: string; played_at: string; placement: number; xp_earned: number;
    rating_delta: number; ranked: boolean; opponents: string[];
  }[];
  achievements: string[];
  challenges: {
    key: string; title: string; progress: number; target: number; reward: number; claimed: boolean;
  }[];
  leaderboard: {
    rank: number; user_id: string; display_name: string; rating: number; matches: number; wins: number;
  }[];
  season_name: string; season_ends_at: string;
  invites: { id: string; lobby_id: string; lobby_name: string; sender_name: string }[];
};
type Snapshot = {
  user: User | null; model: GameViewModel | null; lobbyId: string | null;
  player: number | null; connected: boolean; realtimeConnected: boolean;
  lobbies: LobbySummary[]; lobby: Lobby | null; error: string | null;
  pending: string | null;
  toast: string | null;
  rulePreset: string | null;
  botDifficulty: string | null;
  turnDeadline: number | null;
  syncing: boolean;
  lastSyncedAt: number | null;
  events: Activity[];
  spectating: boolean;
  rematchVotes: { votes: number; needed: number } | null;
  configurationError: string | null;
  hub: PlayerHub | null;
  playerSearch: SocialPlayer[];
  replay: { matchId: string; frames: GameViewModel[] } | null;
  presence: Record<number, "online" | "reconnecting" | "offline">;
};
type ServerMessage =
  | { type: "ready"; user: User; protocol_version: number }
  | { type: "lobby_list"; lobbies: LobbySummary[] }
  | { type: "lobby"; lobby: Lobby }
  | { type: "hub"; hub: PlayerHub }
  | { type: "search_results"; players: SocialPlayer[] }
  | { type: "replay"; match_id: string; frames: GameViewModel[] }
  | { type: "presence"; lobby_id: string; seats: Lobby["seats"] }
  | { type: "join_requested"; lobby_id: string }
  | { type: "join_decision"; lobby_id: string; accepted: boolean }
  | { type: "game_started"; lobby_id: string; player: number; model: GameViewModel; turn_seconds: number }
  | { type: "state"; lobby_id: string; model: GameViewModel; turn_seconds: number }
  | { type: "spectator_started"; lobby_id: string; model: GameViewModel; turn_seconds: number }
  | { type: "activity"; lobby_id: string; event: Activity }
  | { type: "feed"; lobby_id: string; events: Activity[] }
  | { type: "rematch_update"; lobby_id: string; votes: number; needed: number }
  | { type: "ack"; command_id: string }
  | { type: "pong" }
  | {
      type: "error"; command_id: string | null; code: string;
      message: string; recoverable: boolean
    };

function resolveApiUrl() {
  const configured = (import.meta.env.VITE_API_URL as string | undefined)?.trim();
  const candidate = configured || (import.meta.env.DEV ? "http://localhost:8080" : "");
  if (!candidate) return {
    api: null,
    error: "Online play is not configured in this build. Install a release built with the public multiplayer server URL."
  };
  try {
    const url = new URL(candidate);
    const local = ["localhost", "127.0.0.1", "::1"].includes(url.hostname);
    if (!["http:", "https:"].includes(url.protocol) || (import.meta.env.PROD && !local && url.protocol !== "https:"))
      throw new Error("Production multiplayer requires HTTPS");
    return { api: url.toString().replace(/\/$/, ""), error: null };
  } catch {
    return {
      api: null,
      error: "This build contains an invalid multiplayer server URL. Please install a correctly configured release."
    };
  }
}
const serverConfig = resolveApiUrl();
const api = serverConfig.api;
const tokenKey = "ludo-online-token";
const lobbyKey = "ludo-online-lobby";
const spectatorKey = "ludo-online-spectator";

class OnlineStore {
  private listeners = new Set<() => void>();
  private socket: WebSocket | null = null;
  private ably: Realtime | null = null;
  private token = localStorage.getItem(tokenKey);
  private reconnectTimer: number | null = null;
  private heartbeatTimer: number | null = null;
  private reconnectAttempt = 0;
  private commands = new Map<string, string | null>();
  private snapshot: Snapshot = {
    user: null, model: null, lobbyId: localStorage.getItem(lobbyKey), player: null,
    connected: false, realtimeConnected: false, lobbies: [], lobby: null,
    error: null, pending: null
    , toast: null, rulePreset: null, botDifficulty: null, turnDeadline: null,
    syncing: false, lastSyncedAt: null,
    events: [], spectating: localStorage.getItem(spectatorKey) === "true", rematchVotes: null,
    configurationError: serverConfig.error
    , hub: null, playerSearch: [], replay: null, presence: {}
  };

  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };
  getSnapshot = () => this.snapshot;

  async restore() {
    if (!this.token || this.snapshot.user) return;
    if (!api) {
      this.set({ error: serverConfig.error });
      return;
    }
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
    if (!api) {
      const error = new Error(serverConfig.error ?? "Online play is unavailable");
      this.set({ pending: null, error: error.message });
      throw error;
    }
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
  clearToast() { this.set({ toast: null }); }
  listLobbies() { this.send({ type: "list_lobbies" }); }
  getHub() { this.send({ type: "get_hub" }); }
  searchPlayers(query: string) { this.send({ type: "search_players", query }); }
  sendFriendRequest(userId: string) {
    this.command(`friend:${userId}`, { type: "send_friend_request", user_id: userId });
  }
  respondFriendRequest(userId: string, accept: boolean) {
    this.command(`friend:${userId}`, { type: "respond_friend_request", user_id: userId, accept });
  }
  removeFriend(userId: string) {
    this.command(`friend:${userId}`, { type: "remove_friend", user_id: userId });
  }
  inviteFriend(userId: string) {
    if (this.snapshot.lobby)
      this.command(`invite-friend:${userId}`, {
        type: "invite_friend", lobby_id: this.snapshot.lobby.id, user_id: userId
      });
  }
  respondFriendInvite(inviteId: string, accept: boolean) {
    this.command(`friend-invite:${inviteId}`, {
      type: "respond_friend_invite", invite_id: inviteId, accept
    });
  }
  setCosmetics(diceTheme: string, tokenTheme: string) {
    this.command("cosmetics", {
      type: "set_cosmetics", dice_theme: diceTheme, token_theme: tokenTheme
    });
  }
  rankedMatch() { this.command("ranked", { type: "ranked_match" }); }
  getReplay(matchId: string) {
    this.command(`replay:${matchId}`, { type: "get_replay", match_id: matchId });
  }
  closeReplay() { this.set({ replay: null }); }
  createLobby(options: {
    name: string; rule_preset: string; bot_difficulty: string; is_public: boolean;
    turn_seconds: number
  }) {
    this.command("create", { type: "create_lobby", ...options });
  }
  requestJoin(lobbyId: string) {
    this.command(`join:${lobbyId}`, { type: "request_join", lobby_id: lobbyId });
  }
  joinByCode(inviteCode: string) {
    this.command("invite", { type: "join_by_code", invite_code: inviteCode });
  }
  cancelJoin(lobbyId: string) {
    this.command(`cancel:${lobbyId}`, { type: "cancel_join", lobby_id: lobbyId });
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
  setReady(ready: boolean) {
    if (this.snapshot.lobby)
      this.command("ready", { type: "set_ready", lobby_id: this.snapshot.lobby.id, ready });
  }
  kickPlayer(userId: string) {
    if (this.snapshot.lobby)
      this.command(`kick:${userId}`, {
        type: "kick_player", lobby_id: this.snapshot.lobby.id, user_id: userId
      });
  }
  updateLobby(options: {
    rule_preset: string; bot_difficulty: string; is_public: boolean; turn_seconds: number;
    rematch_mode?: string
  }) {
    if (this.snapshot.lobby)
      this.command("settings", {
        type: "update_lobby", lobby_id: this.snapshot.lobby.id,
        rematch_mode: options.rematch_mode ?? this.snapshot.lobby.rematch_mode, ...options
      });
  }
  quickMatch(rulePreset: string, botDifficulty: string) {
    this.command("quick", {
      type: "quick_match", rule_preset: rulePreset, bot_difficulty: botDifficulty
    });
  }
  spectate(lobbyId: string) {
    this.command(`spectate:${lobbyId}`, { type: "spectate", lobby_id: lobbyId });
  }
  chat(body: string) {
    if (this.snapshot.lobbyId)
      this.send({ type: "chat", lobby_id: this.snapshot.lobbyId, body });
  }
  react(emoji: string) {
    if (this.snapshot.lobbyId)
      this.send({ type: "react", lobby_id: this.snapshot.lobbyId, emoji });
  }
  voteRematch() {
    if (this.snapshot.lobbyId)
      this.command("rematch", { type: "vote_rematch", lobby_id: this.snapshot.lobbyId });
  }
  resync() {
    if (!this.snapshot.lobbyId) return;
    this.set({ syncing: true, error: null });
    if (!this.send({ type: "sync", lobby_id: this.snapshot.lobbyId }))
      this.set({ syncing: false });
  }
  async copyInvite() {
    const code = this.snapshot.lobby?.invite_code;
    if (!code) return;
    const url = new URL(location.href);
    url.searchParams.set("invite", code);
    const share = { title: "Join my Ludo game", text: "Take a seat at my Ludo table.", url: url.toString() };
    if (navigator.share && navigator.canShare?.(share)) {
      await navigator.share(share);
      this.set({ toast: "Invite shared." });
      return;
    }
    await navigator.clipboard.writeText(url.toString());
    this.set({ toast: "Invite link copied." });
  }
  showLocal() {
    localStorage.removeItem(lobbyKey);
    localStorage.removeItem(spectatorKey);
    this.set({
      model: null, lobbyId: null, player: null, lobby: null, error: null,
      spectating: false, events: [], rematchVotes: null
    });
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
    if (!api || !this.token || this.socket?.readyState === WebSocket.OPEN
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
      this.set({ connected: true, error: null, syncing: Boolean(this.snapshot.lobbyId) });
      this.connectAbly();
      if (this.heartbeatTimer !== null) window.clearInterval(this.heartbeatTimer);
      this.heartbeatTimer = window.setInterval(() => {
        if (this.socket?.readyState === WebSocket.OPEN) this.send({ type: "ping" });
      }, 15_000);
      if (this.snapshot.lobbyId)
        this.send({
          type: this.snapshot.spectating ? "spectate" : "sync",
          lobby_id: this.snapshot.lobbyId
        });
      const invite = new URL(location.href).searchParams.get("invite");
      if (invite) {
        this.joinByCode(invite);
        const clean = new URL(location.href);
        clean.searchParams.delete("invite");
        history.replaceState(null, "", clean);
      }
    };
    this.socket.onclose = () => {
      if (this.heartbeatTimer !== null) window.clearInterval(this.heartbeatTimer);
      this.heartbeatTimer = null;
      this.socket = null;
      this.set({ connected: false, pending: null, syncing: Boolean(this.snapshot.lobbyId) });
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
    if (!api || !this.token || !this.snapshot.user || this.ably) return;
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
    if (value.type === "ready" || value.type === "pong") return;
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
      if (this.snapshot.model?.winner !== null && this.snapshot.model?.winner !== undefined) {
        localStorage.removeItem(lobbyKey);
        this.set({
          lobby: value.lobby, model: null, lobbyId: null, player: null,
          pending: null, error: null, rematchVotes: null
        });
      } else {
        this.set({ lobby: value.lobby, pending: null, error: null });
      }
      return;
    }
    if (value.type === "hub") {
      this.set({ hub: value.hub, pending: null });
      return;
    }
    if (value.type === "search_results") {
      this.set({ playerSearch: value.players });
      return;
    }
    if (value.type === "replay") {
      this.set({ replay: { matchId: value.match_id, frames: value.frames }, pending: null });
      return;
    }
    if (value.type === "presence") {
      if (this.snapshot.lobbyId && value.lobby_id !== this.snapshot.lobbyId) return;
      this.set({
        presence: Object.fromEntries(value.seats.filter(seat => seat.presence !== "bot")
          .map(seat => [seat.seat, seat.presence])) as Snapshot["presence"]
      });
      return;
    }
    if (value.type === "join_requested") {
      this.set({ pending: null, error: null, toast: "Join request sent to the host." });
      return;
    }
    if (value.type === "join_decision") {
      this.set({
        pending: null,
        error: null,
        toast: value.accepted ? "Your seat was accepted. Welcome to the table!" : "The host declined your join request."
      });
      return;
    }
    if (value.type === "game_started") {
      localStorage.setItem(lobbyKey, value.lobby_id);
      localStorage.removeItem(spectatorKey);
      this.set({
        lobby: null, lobbyId: value.lobby_id, player: value.player,
        model: value.model, pending: null, error: null,
        rulePreset: this.snapshot.lobby?.rule_preset ?? this.snapshot.rulePreset,
        botDifficulty: this.snapshot.lobby?.bot_difficulty ?? this.snapshot.botDifficulty,
        turnDeadline: Date.now() + value.turn_seconds * 1000,
        spectating: false, events: [], syncing: false, lastSyncedAt: Date.now()
      });
      return;
    }
    if (value.type === "state") {
      if (this.snapshot.lobbyId && value.lobby_id !== this.snapshot.lobbyId) return;
      if (this.snapshot.model && value.model.revision < this.snapshot.model.revision) return;
      this.set({
        model: value.model, pending: null, error: null,
        turnDeadline: Date.now() + value.turn_seconds * 1000,
        syncing: false, lastSyncedAt: Date.now()
      });
      return;
    }
    if (value.type === "spectator_started") {
      localStorage.setItem(lobbyKey, value.lobby_id);
      localStorage.setItem(spectatorKey, "true");
      this.set({
        lobby: null, lobbyId: value.lobby_id, player: null, model: value.model,
        spectating: true, events: [], pending: null, error: null,
        turnDeadline: Date.now() + value.turn_seconds * 1000,
        syncing: false, lastSyncedAt: Date.now()
      });
      return;
    }
    if (value.type === "feed") {
      this.set({ events: value.events });
      return;
    }
    if (value.type === "activity") {
      if (value.lobby_id === this.snapshot.lobbyId)
        this.set({ events: [...this.snapshot.events.slice(-39), value.event] });
      return;
    }
    if (value.type === "rematch_update") {
      this.set({ rematchVotes: { votes: value.votes, needed: value.needed } });
      return;
    }
    if (value.command_id) this.commands.delete(value.command_id);
    this.set({ error: value.message, pending: this.commands.size === 0 ? null : this.snapshot.pending });
    if (value.code === "stale_revision" && this.snapshot.lobbyId) this.resync();
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
    this.socket.send(JSON.stringify({
      command_id: commandId, protocol_version: 1, ...value as object
    }));
    return true;
  }
  private resetSession() {
    if (this.reconnectTimer !== null) window.clearTimeout(this.reconnectTimer);
    if (this.heartbeatTimer !== null) window.clearInterval(this.heartbeatTimer);
    this.reconnectTimer = null;
    this.heartbeatTimer = null;
    this.socket?.close();
    this.socket = null;
    this.ably?.close();
    this.ably = null;
    this.commands.clear();
    this.token = null;
    localStorage.removeItem(tokenKey);
    localStorage.removeItem(lobbyKey);
    localStorage.removeItem(spectatorKey);
    this.snapshot = {
      user: null, model: null, lobbyId: null, player: null, connected: false,
      realtimeConnected: false, lobbies: [], lobby: null, error: null, pending: null
      , toast: null, rulePreset: null, botDifficulty: null, turnDeadline: null,
      syncing: false, lastSyncedAt: null,
      events: [], spectating: false, rematchVotes: null,
      configurationError: serverConfig.error, hub: null, playerSearch: [], replay: null, presence: {}
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
  if (error instanceof TypeError && /fetch|network|load/i.test(error.message))
    return "Could not reach the multiplayer server. Check your internet connection or install the latest configured release.";
  return error instanceof Error ? error.message : String(error);
}

export const onlineStore = new OnlineStore();
export function useOnlineStore() {
  return useSyncExternalStore(onlineStore.subscribe, onlineStore.getSnapshot);
}
