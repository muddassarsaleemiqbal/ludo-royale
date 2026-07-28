import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Bot,
  Check,
  CheckCircle2,
  ChevronRight,
  Clock3,
  Crown,
  Copy,
  Eye,
  Gamepad2,
  Globe2,
  LockKeyhole,
  MessageCircle,
  Plus,
  RefreshCw,
  Settings,
  Shield,
  Sparkles,
  Trophy,
  UserX,
  UsersRound,
  Volume2
} from "lucide-react";
import { gameStore, useGameStore } from "./game/store";
import { Button } from "./components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  DialogTrigger
} from "./components/ui/dialog";
import { LudoBoard } from "./components/game/board";
import { PlayerCard } from "./components/game/player-card";
import { Dice } from "./components/game/dice";
import { onlineStore, useOnlineStore } from "./game/online";
import { gameAudio } from "./game/audio";
import { preferenceStore, usePreferences } from "./game/preferences";

function LoadingScreen() {
  return (
    <main className="loading-screen">
      <div className="brand-mark"><span>♟</span></div>
      <h1>Ludo Royale</h1>
      <p>Preparing the board…</p>
      <div className="loading-track"><span /></div>
    </main>
  );
}

function OnlineDialog() {
  const online = useOnlineStore();
  const [register, setRegister] = useState(false);
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);
  if (!online.user) {
    return (
      <DialogContent>
        <DialogTitle>{register ? "Create your player account" : "Login for online play"}</DialogTitle>
        <DialogDescription>Accounts are required for fair matches and reconnection.</DialogDescription>
        <form className="online-form" onSubmit={(event) => {
          event.preventDefault();
          setError(null);
          void onlineStore.authenticate(register ? "register" : "login", email, password, name)
            .catch((cause) => setError(String(cause)));
        }}>
          {register && <label>Display name<input autoComplete="nickname" value={name} onChange={(e) => setName(e.target.value)} required /></label>}
          <label>Email<input type="email" autoComplete="email" value={email} onChange={(e) => setEmail(e.target.value)} required /></label>
          <label>Password<input type="password" autoComplete={register ? "new-password" : "current-password"} value={password} onChange={(e) => setPassword(e.target.value)} minLength={10} required /></label>
          {(error ?? online.error) && <p className="online-error">{error ?? online.error}</p>}
          <Button type="submit">{register ? "Create account" : "Login"}</Button>
          <Button type="button" variant="ghost" onClick={() => setRegister(!register)}>
            {register ? "I already have an account" : "Create a new account"}
          </Button>
        </form>
      </DialogContent>
    );
  }
  return (
    <DialogContent>
      <DialogTitle>Your online profile</DialogTitle>
      <DialogDescription>Signed in as {online.user.display_name}. Your active games reconnect automatically.</DialogDescription>
      <div className="online-lobby">
        <p>{online.connected ? "Connected to the royal tables." : "Reconnecting…"}</p>
        <Button variant="ghost" onClick={() => onlineStore.logout()}>Log out</Button>
      </div>
    </DialogContent>
  );
}

const presets = [
  { id: "classic", name: "Classic", detail: "Safe stars, blockades and bonus turns", time: "35–50 min" },
  { id: "quick", name: "Quick play", detail: "Faster routes and forgiving home rolls", time: "15–25 min" },
  { id: "tournament", name: "Tournament", detail: "Rank every player with competitive rules", time: "45–60 min" }
];

type LocalSetup = {
  preset: "Classic" | "Quick" | "Tournament";
  botDifficulty: "Easy" | "Medium" | "Hard";
};

function SetupScreen({ onLocal }: { onLocal: (setup: LocalSetup) => void }) {
  const online = useOnlineStore();
  const [mode, setMode] = useState<"local" | "online">("local");
  const [preset, setPreset] = useState("classic");
  const [difficulty, setDifficulty] = useState("medium");
  const [tableName, setTableName] = useState("");
  const [confirmLeave, setConfirmLeave] = useState(false);
  const [isPublic, setIsPublic] = useState(true);
  const [turnSeconds, setTurnSeconds] = useState(30);
  const [inviteCode, setInviteCode] = useState("");
  const [rulesFilter, setRulesFilter] = useState("all");
  const [occupancyFilter, setOccupancyFilter] = useState("all");
  const localSetup = (): LocalSetup => ({
    preset: (preset[0]?.toUpperCase() + preset.slice(1)) as LocalSetup["preset"],
    botDifficulty: (difficulty[0]?.toUpperCase() + difficulty.slice(1)) as LocalSetup["botDifficulty"]
  });

  return (
    <main className="setup-shell">
      <div className="setup-orb orb-a" /><div className="setup-orb orb-b" />
      <header className="setup-header">
        <div className="brand"><div className="brand-mark small"><span>♟</span></div><div><strong>Ludo Royale</strong><span>Choose your table</span></div></div>
        <div className="setup-profile">
          {online.user ? <><span className={online.connected && online.realtimeConnected ? "online-dot" : ""} />{online.user.display_name}</> : <Dialog><DialogTrigger asChild><Button variant="secondary">Sign in</Button></DialogTrigger><OnlineDialog /></Dialog>}
        </div>
      </header>

      <section className="setup-content">
        <div className="setup-intro">
          <span className="eyebrow"><Crown /> The royal board awaits</span>
          <h1>Set the rules.<br /><em>Claim the crown.</em></h1>
          <p>Play your way with three clever rivals, or open a table and let friends take their seats.</p>
        </div>

        <div className="setup-card">
          <div className="setup-tabs">
            <button className={mode === "local" ? "active" : ""} onClick={() => setMode("local")}><Gamepad2 /> Solo game</button>
            <button className={mode === "online" ? "active" : ""} onClick={() => setMode("online")}><Globe2 /> Online tables</button>
          </div>

          {mode === "local" ? <>
            <div className="section-label"><span>1</span><div><strong>Choose a ruleset</strong><small>You can change this before every match</small></div></div>
            <div className="preset-grid">
              {presets.map((item) => <button key={item.id} className={preset === item.id ? "active" : ""} onClick={() => setPreset(item.id)}>
                <span className="preset-check">{preset === item.id && <Check />}</span><strong>{item.name}</strong><small>{item.detail}</small><em><Clock3 /> {item.time}</em>
              </button>)}
            </div>
            <div className="option-row">
              <div><strong>AI difficulty</strong><small>Applies to all three opponents</small></div>
              <div className="segmented">{["easy","medium","hard"].map(level => <button key={level} className={difficulty === level ? "active" : ""} onClick={() => setDifficulty(level)}>{level}</button>)}</div>
            </div>
            <Button size="lg" className="setup-primary" onClick={() => onLocal(localSetup())}>Start solo game <ChevronRight /></Button>
          </> : !online.user ? <div className="signin-prompt">
            <LockKeyhole /><h2>Sign in to join the tables</h2><p>An account keeps games fair and lets you reconnect on any device.</p>
            <Dialog><DialogTrigger asChild><Button>Sign in or create account</Button></DialogTrigger><OnlineDialog /></Dialog>
          </div> : online.lobby ? <div className="lobby-room">
            <div className="lobby-title"><div><span>Waiting room · {online.lobby.is_public ? "Public" : "Private"}</span><h2>{online.lobby.name}</h2></div><small>{online.lobby.rule_preset} · {online.lobby.bot_difficulty} AI · {online.lobby.turn_seconds}s</small></div>
            <div className="invite-strip"><code>{online.lobby.invite_code}</code><Button size="sm" variant="secondary" onClick={() => void onlineStore.copyInvite()}><Copy /> Copy invite</Button></div>
            <div className="seat-list">{online.lobby.seats.map((seat, index) => <div className={seat.is_bot ? "bot-seat" : "human-seat"} key={seat.seat}><span>{index + 1}</span><div><strong>{seat.name}</strong><small>{seat.is_bot ? "AI fills this seat" : `${seat.ready ? "Ready" : "Not ready"} · ${seat.presence}`}</small></div>{seat.ready && !seat.is_bot && <CheckCircle2 className="ready-icon" />}{online.lobby?.host_user_id === online.user?.id && seat.user_id && seat.user_id !== online.user?.id && <button className="seat-kick" aria-label={`Remove ${seat.name}`} onClick={() => onlineStore.kickPlayer(seat.user_id!)}><UserX /></button>}{online.lobby?.host_user_id === seat.user_id && <Crown />}</div>)}</div>
            {online.lobby.host_user_id === online.user.id && online.lobby.requests.length > 0 && <div className="request-list"><strong>Join requests</strong>{online.lobby.requests.map(request => <div key={request.id}><span>{request.display_name}</span><Button size="sm" disabled={online.pending === `request:${request.id}`} onClick={() => onlineStore.respondJoin(request.id, true)}>{online.pending === `request:${request.id}` ? "Saving…" : "Accept"}</Button><Button size="sm" variant="ghost" disabled={online.pending === `request:${request.id}`} onClick={() => onlineStore.respondJoin(request.id, false)}>Decline</Button></div>)}</div>}
            {online.lobby.host_user_id === online.user.id && <div className="host-controls"><select aria-label="Lobby rules" value={online.lobby.rule_preset} onChange={event => onlineStore.updateLobby({ rule_preset:event.target.value, bot_difficulty:online.lobby!.bot_difficulty, is_public:online.lobby!.is_public, turn_seconds:online.lobby!.turn_seconds })}>{presets.map(item => <option key={item.id} value={item.id}>{item.name}</option>)}</select><select aria-label="Lobby AI difficulty" value={online.lobby.bot_difficulty} onChange={event => onlineStore.updateLobby({ rule_preset:online.lobby!.rule_preset, bot_difficulty:event.target.value, is_public:online.lobby!.is_public, turn_seconds:online.lobby!.turn_seconds })}>{["easy","medium","hard"].map(level => <option key={level} value={level}>{level} AI</option>)}</select><select aria-label="Lobby turn timer" value={online.lobby.turn_seconds} onChange={event => onlineStore.updateLobby({ rule_preset:online.lobby!.rule_preset, bot_difficulty:online.lobby!.bot_difficulty, is_public:online.lobby!.is_public, turn_seconds:Number(event.target.value) })}>{[15,30,45,60].map(seconds => <option key={seconds} value={seconds}>{seconds}s turns</option>)}</select><button role="switch" aria-checked={online.lobby.is_public} onClick={() => onlineStore.updateLobby({ rule_preset:online.lobby!.rule_preset, bot_difficulty:online.lobby!.bot_difficulty, is_public:!online.lobby!.is_public, turn_seconds:online.lobby!.turn_seconds })}>{online.lobby.is_public ? "Public table" : "Private table"}</button></div>}
            <div className="lobby-actions">{online.lobby.host_user_id === online.user.id ? <><Button variant="secondary" onClick={() => setConfirmLeave(true)}>Transfer host / close</Button><Button size="lg" disabled={online.pending === "start" || online.lobby.seats.some(seat => !seat.is_bot && seat.user_id !== online.user!.id && !seat.ready)} onClick={() => onlineStore.startGame()}>{online.pending === "start" ? "Starting…" : "Start with this lineup"}</Button></> : <><Button variant="secondary" onClick={() => onlineStore.setReady(!online.lobby!.seats.find(seat => seat.user_id === online.user!.id)?.ready)}>{online.lobby.seats.find(seat => seat.user_id === online.user!.id)?.ready ? "Not ready" : "I'm ready"}</Button><Button variant="ghost" onClick={() => setConfirmLeave(true)}>Leave table</Button></>}</div>
          </div> : <div className="table-browser">
            <div className="browser-heading"><div><strong>Open tables</strong><small>Join friends or watch live games</small></div><Button size="icon" variant="ghost" onClick={() => onlineStore.listLobbies()} aria-label="Refresh tables"><RefreshCw /></Button></div>
            <div className="lobby-filters"><select aria-label="Filter by rules" value={rulesFilter} onChange={event => setRulesFilter(event.target.value)}><option value="all">All rules</option>{presets.map(item => <option key={item.id} value={item.id}>{item.name}</option>)}</select><select aria-label="Filter by occupancy" value={occupancyFilter} onChange={event => setOccupancyFilter(event.target.value)}><option value="all">Any seats</option><option value="open">Open seats</option><option value="nearly">Nearly full</option></select></div>
            <div className="table-list">{online.lobbies.filter(lobby => (rulesFilter === "all" || lobby.rule_preset === rulesFilter) && (occupancyFilter === "all" || (occupancyFilter === "open" ? lobby.human_players < 4 : lobby.human_players >= 3))).length ? online.lobbies.filter(lobby => (rulesFilter === "all" || lobby.rule_preset === rulesFilter) && (occupancyFilter === "all" || (occupancyFilter === "open" ? lobby.human_players < 4 : lobby.human_players >= 3))).map(lobby => <div className="table-row" key={lobby.id}>
              <div className="table-icon"><Crown /></div><div><strong>{lobby.name}</strong><small>Hosted by {lobby.host_name} · {lobby.rule_preset}</small></div>
              <span>{lobby.human_players}/4</span>
              {lobby.status === "playing" ? <Button size="sm" variant="secondary" onClick={() => onlineStore.spectate(lobby.id)}><Eye /> Watch</Button> : lobby.is_host ? <Button size="sm" disabled>Hosting</Button> : lobby.requested ? <Button size="sm" variant="ghost" onClick={() => onlineStore.cancelJoin(lobby.id)}>Cancel</Button> : <Button size="sm" variant="secondary" disabled={online.pending === `join:${lobby.id}`} onClick={() => onlineStore.requestJoin(lobby.id)}>{online.pending === `join:${lobby.id}` ? "Sending…" : "Request seat"}</Button>}
            </div>) : <div className="empty-tables"><Globe2 /><strong>No open tables yet</strong><span>Be the first host online.</span></div>}</div>
            <div className="match-actions"><Button variant="secondary" onClick={() => onlineStore.quickMatch(preset,difficulty)}>Quick match</Button><div><input aria-label="Invite code" placeholder="Invite code" value={inviteCode} onChange={event => setInviteCode(event.target.value.toUpperCase())} maxLength={8}/><Button size="sm" onClick={() => onlineStore.joinByCode(inviteCode)}>Join invite</Button></div></div>
            <div className="create-table">
              <div className="section-label"><span><Plus /></span><div><strong>Host a new table</strong><small>Three AI players fill empty seats</small></div></div>
              <label className="field-label">Table name<input value={tableName} onChange={e => setTableName(e.target.value)} placeholder={`${online.user.display_name}'s table`} maxLength={40} /></label>
              <div className="create-options"><label>Rules<select value={preset} onChange={e => setPreset(e.target.value)}>{presets.map(p => <option value={p.id} key={p.id}>{p.name}</option>)}</select></label><label>AI difficulty<select value={difficulty} onChange={e => setDifficulty(e.target.value)}><option value="easy">Easy AI</option><option value="medium">Medium AI</option><option value="hard">Hard AI</option></select></label><label>Turn timer<select value={turnSeconds} onChange={event => setTurnSeconds(Number(event.target.value))}>{[15,30,45,60].map(seconds => <option key={seconds} value={seconds}>{seconds} seconds</option>)}</select></label><button className="visibility-toggle" role="switch" aria-checked={isPublic} onClick={() => setIsPublic(!isPublic)}>{isPublic ? "Public" : "Private"}</button></div>
              <Button disabled={online.pending === "create"} onClick={() => { onlineStore.createLobby({ name: tableName, rule_preset: preset, bot_difficulty: difficulty, is_public:isPublic, turn_seconds:turnSeconds }); }}>{online.pending === "create" ? "Creating table…" : `Create ${isPublic ? "public" : "private"} table`}</Button>
            </div>
          </div>}
          {online.error && <div className="online-error" role="alert"><span>{online.error}</span><button onClick={() => onlineStore.clearError()} aria-label="Dismiss error">×</button></div>}
          {online.toast && <div className="game-toast" role="status"><span>{online.toast}</span><button onClick={() => onlineStore.clearToast()} aria-label="Dismiss notification">×</button></div>}
        </div>
      </section>
      <Dialog open={confirmLeave} onOpenChange={setConfirmLeave}>
        <DialogContent>
          <DialogTitle>Leave this table?</DialogTitle>
          <DialogDescription>{online.lobby?.host_user_id === online.user?.id ? "Hosting will transfer to the longest-waiting connected player. If nobody remains, the table will close." : "Your seat will return to an AI player. You can request another seat later."}</DialogDescription>
          <div className="dialog-actions">
            <Button variant="ghost" onClick={() => setConfirmLeave(false)}>Stay</Button>
            <Button onClick={() => { onlineStore.leaveLobby(); setConfirmLeave(false); }}>Leave table</Button>
          </div>
        </DialogContent>
      </Dialog>
    </main>
  );
}

function GameHeader({ platform, onlineActive, onLocal }: {
  platform: "web" | "native"; onlineActive: boolean; onLocal: () => void;
}) {
  const preferences = usePreferences();
  return (
    <header className="app-header">
      <div className="brand">
        <div className="brand-mark small"><span>♟</span></div>
        <div>
          <strong>Ludo Royale</strong>
          <span>{platform === "native" ? "Native edition" : "WebAssembly edition"}</span>
        </div>
      </div>
      <nav className="mode-pills" aria-label="Game mode">
        <button className={onlineActive ? "" : "active"} onClick={onLocal}><Gamepad2 /> Local</button>
        <Dialog>
          <DialogTrigger asChild>
            <button className={onlineActive ? "active" : ""}><Globe2 /> Online</button>
          </DialogTrigger>
          <OnlineDialog />
        </Dialog>
        <button disabled title="Tournaments are planned"><Trophy /> Tournaments</button>
      </nav>
      <Dialog>
        <DialogTrigger asChild>
          <Button variant="secondary" size="icon" aria-label="Settings">
            <Settings className="size-4" />
          </Button>
        </DialogTrigger>
        <DialogContent>
          <DialogTitle className="text-xl font-bold">Game settings</DialogTitle>
          <DialogDescription className="mt-1 text-sm text-stone-400">
            Your preferences stay on this device.
          </DialogDescription>
          <div className="settings-list">
            <button role="switch" aria-checked={preferences.sound} onClick={() => {
              const enabled = !preferences.sound;
              preferenceStore.set("sound", enabled);
              if (enabled) gameAudio.play("notification", true);
            }}><Volume2 /> Sound effects <span>{preferences.sound ? "On" : "Muted"}</span></button>
            <button role="switch" aria-checked={preferences.motion} onClick={() => preferenceStore.set("motion", !preferences.motion)}><Sparkles /> Motion effects <span>{preferences.motion ? "Full" : "Reduced"}</span></button>
            <button role="switch" aria-checked={preferences.safeHints} onClick={() => preferenceStore.set("safeHints", !preferences.safeHints)}><Shield /> Safe-cell hints <span>{preferences.safeHints ? "On" : "Off"}</span></button>
          </div>
        </DialogContent>
      </Dialog>
    </header>
  );
}

export default function App() {
  const { model, starting, fatalError, platform } = useGameStore();
  const online = useOnlineStore();
  const [confirmNew, setConfirmNew] = useState(false);
  const [inGame, setInGame] = useState(false);
  const [activeSetup, setActiveSetup] = useState<LocalSetup | null>(null);
  const [secondsLeft, setSecondsLeft] = useState<number | null>(null);
  const [chatText, setChatText] = useState("");
  const preferences = usePreferences();
  const previousModel = useRef<typeof model>(null);

  useEffect(() => {
    void gameStore.initialize();
    void onlineStore.restore();
  }, []);

  const activeModel = online.model ?? model;
  const current = activeModel?.players.find((player) => player.active);
  const status = activeModel?.error ?? activeModel?.status ?? "";
  const turnStatus = !online.connected && online.model
    ? "Reconnecting — your game is safe"
    : activeModel?.winner !== null && activeModel?.winner !== undefined
      ? `${activeModel.players.find(player => player.id === activeModel.winner)?.name ?? "A player"} wins!`
      : activeModel?.busy
        ? "AI is thinking…"
        : online.model && current?.id !== online.player
          ? `Waiting for ${current?.name ?? "the next player"}`
          : "Your turn";
  const boardTokens = useMemo(
    () => (activeModel?.tokens ?? []).map((token) => ({
      ...token,
      selectable: token.selectable && (!online.model || current?.id === online.player)
    })),
    [activeModel?.tokens, current?.id, online.model, online.player]
  );
  const handleTokenSelect = useCallback((token: number) => {
    gameAudio.unlock();
    if (online.model) {
      onlineStore.move(token);
      return;
    }
    void gameStore.dispatch({ SelectToken: token });
  }, [online.model]);
  const handleRoll = useCallback(() => {
    gameAudio.unlock();
    if (online.model) {
      onlineStore.roll();
      return;
    }
    void gameStore.dispatch("Roll");
  }, [online.model]);

  useEffect(() => {
    if (!activeModel) return;
    const previous = previousModel.current;
    if (previous && activeModel.revision > previous.revision) {
      if (activeModel.winner !== null && previous.winner === null) {
        gameAudio.play("victory", preferences.sound);
      } else if (activeModel.dice !== previous.dice && activeModel.dice !== null) {
        gameAudio.play("roll", preferences.sound);
      } else if (activeModel.tokens.some((token, index) =>
        token.position === "Yard" && previous.tokens[index]?.position !== "Yard"
      )) {
        gameAudio.play("capture", preferences.sound);
      } else if (activeModel.tokens.some((token, index) =>
        JSON.stringify(token.position) !== JSON.stringify(previous.tokens[index]?.position)
      )) {
        gameAudio.play("move", preferences.sound);
      } else if (current?.id !== previous.players.find(player => player.active)?.id) {
        gameAudio.play("turn", preferences.sound);
      }
    }
    previousModel.current = activeModel;
  }, [activeModel, current?.id, preferences.sound]);

  useEffect(() => {
    if (!online.turnDeadline) {
      setSecondsLeft(null);
      return;
    }
    const update = () => setSecondsLeft(Math.max(0, Math.ceil((online.turnDeadline! - Date.now()) / 1000)));
    update();
    const timer = window.setInterval(update, 250);
    return () => window.clearInterval(timer);
  }, [online.turnDeadline]);

  useEffect(() => {
    if (online.toast) gameAudio.play("notification", preferences.sound);
  }, [online.toast, preferences.sound]);

  if (starting) return <LoadingScreen />;
  if (!activeModel) {
    return (
      <main className="fatal-screen">
        <h1>Unable to start Ludo</h1>
        <p>{fatalError ?? "The game engine did not return a board."}</p>
        <Button onClick={() => location.reload()}>Try again</Button>
      </main>
    );
  }

  if (!inGame && !online.model) {
    return <SetupScreen onLocal={(setup) => {
      setActiveSetup(setup);
      void gameStore.dispatch({
        NewGameWith: {
          preset: setup.preset,
          bot_difficulty: setup.botDifficulty
        }
      });
      setInGame(true);
    }} />;
  }

  return (
    <main className="app-shell">
      <div className="ambient ambient-one" />
      <div className="ambient ambient-two" />
      <GameHeader platform={platform} onlineActive={Boolean(online.model)} onLocal={() => {
        onlineStore.showLocal();
        setInGame(false);
      }} />

      <div className="turn-banner" role="status" aria-live="polite">
        {online.spectating ? "Spectating live" : turnStatus}{secondsLeft !== null && activeModel.winner === null ? ` · ${secondsLeft}s` : ""}
      </div>
      <section className="game-layout">
        <aside className="left-panel">
          <div className="panel-heading">
            <div><UsersRound /><span>Players</span></div>
            <small>{online.rulePreset ?? activeSetup?.preset ?? "Classic"} • {online.botDifficulty ?? activeSetup?.botDifficulty ?? "Medium"} AI</small>
          </div>
          <div className="player-list">
            {activeModel.players.map((player) => (
              <PlayerCard key={player.id} player={player} />
            ))}
          </div>
          <div className="game-tip">
            <Shield />
            <div>
              <strong>Safe squares</strong>
              <span title="Tokens on shield cells cannot be captured">Shield marks protect your token from capture.</span>
            </div>
          </div>
          {online.model && <div className="social-panel">
            <div className="social-title"><MessageCircle /><strong>Match feed</strong>{online.spectating && <span>Watching</span>}</div>
            <div className="event-feed" aria-live="polite">{online.events.length ? online.events.map(event => <div className={`event-${event.kind}`} key={event.id}>{event.message}</div>) : <span>Moves, reactions, and chat appear here.</span>}</div>
            <div className="reaction-row">{["👍","👏","😮","😂","🔥","👑"].map(emoji => <button key={emoji} aria-label={`React ${emoji}`} onClick={() => onlineStore.react(emoji)}>{emoji}</button>)}</div>
            <form className="chat-form" onSubmit={event => { event.preventDefault(); if (chatText.trim()) { onlineStore.chat(chatText); setChatText(""); } }}><label className="sr-only" htmlFor="match-chat">Match chat</label><input id="match-chat" value={chatText} onChange={event => setChatText(event.target.value)} maxLength={240} placeholder="Say something…" /><button type="submit" aria-label="Send chat">Send</button></form>
          </div>}
        </aside>

        <section className="board-section">
          <div className="mobile-players">
            {activeModel.players.map((player) => (
              <PlayerCard key={player.id} player={player} compact />
            ))}
          </div>
          <LudoBoard
            tokens={boardTokens}
            onSelect={handleTokenSelect}
            showSafeCells={preferences.safeHints}
          />
          <div className="mobile-status" aria-live="polite">{turnStatus} · {status}</div>
        </section>

        <aside className="right-panel">
          <div className="turn-label">
            <span className={`color-dot ${current?.color.toLowerCase()}`} />
            {current?.name ?? "Current player"}
          </div>
          <div className="dice-stage">
            <div className="dice-light" />
            <Dice value={activeModel.dice} busy={activeModel.busy} />
          </div>
          <Button
            size="lg"
            className="roll-button"
            disabled={online.spectating || !activeModel.can_roll || Boolean(online.model && current?.id !== online.player)}
            onClick={handleRoll}
          >
            {activeModel.busy
              ? <><Bot className="size-5" /> AI thinking…</>
              : activeModel.can_roll
                ? "Roll dice"
                : activeModel.human_turn
                  ? "Choose a token"
                  : "Waiting for AI"}
          </Button>
          <div className="status-card">
            <span>Match status</span>
            <strong>{status}</strong>
          </div>
          <div className="rule-note" title="A six releases a token from its yard. Captures grant a bonus turn.">
            Roll a six to enter the board. Capture rivals to earn another turn.
          </div>
          <Dialog open={confirmNew} onOpenChange={setConfirmNew}>
            <DialogTrigger asChild>
              <Button variant="secondary" className="w-full">New game</Button>
            </DialogTrigger>
            <DialogContent>
              <DialogTitle className="text-xl font-bold">Start a new game?</DialogTitle>
              <DialogDescription className="mt-2 text-stone-400">
                Your current match progress will be replaced.
              </DialogDescription>
              <div className="mt-6 flex justify-end gap-3">
                <Button variant="ghost" onClick={() => setConfirmNew(false)}>Keep playing</Button>
                <Button onClick={() => {
                  setConfirmNew(false);
                  onlineStore.showLocal();
                  setInGame(false);
                }}>Return to setup</Button>
              </div>
            </DialogContent>
          </Dialog>
        </aside>
      </section>
      {activeModel.winner !== null && <div className="results-overlay" role="dialog" aria-modal="true" aria-labelledby="results-title">
        <div className="results-card">
          <Trophy />
          <span>Match complete</span>
          <h2 id="results-title">{activeModel.players.find(player => player.id === activeModel.winner)?.name} takes the crown!</h2>
          <ol>{[...activeModel.players].sort((a, b) => b.finished - a.finished).map(player => <li key={player.id}><strong>{player.name}</strong><span>{player.finished}/4 home</span></li>)}</ol>
          {online.model && !online.spectating && <Button disabled={online.pending === "rematch"} onClick={() => onlineStore.voteRematch()}>{online.rematchVotes ? `Rematch ${online.rematchVotes.votes}/${online.rematchVotes.needed}` : "Vote for rematch"}</Button>}
          <Button variant="secondary" onClick={() => { onlineStore.showLocal(); setInGame(false); }}>Return to setup</Button>
        </div>
      </div>}
    </main>
  );
}
