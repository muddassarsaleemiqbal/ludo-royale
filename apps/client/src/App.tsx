import { useCallback, useEffect, useMemo, useState } from "react";
import {
  Bot,
  Check,
  ChevronRight,
  Clock3,
  Crown,
  Gamepad2,
  Globe2,
  LockKeyhole,
  Plus,
  RefreshCw,
  Settings,
  Shield,
  Sparkles,
  Trophy,
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
          {register && <input placeholder="Display name" value={name} onChange={(e) => setName(e.target.value)} required />}
          <input type="email" placeholder="Email" value={email} onChange={(e) => setEmail(e.target.value)} required />
          <input type="password" placeholder="Password (10+ characters)" value={password} onChange={(e) => setPassword(e.target.value)} minLength={10} required />
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
            <div className="lobby-title"><div><span>Waiting room</span><h2>{online.lobby.name}</h2></div><small>{online.lobby.rule_preset} · {online.lobby.bot_difficulty} AI</small></div>
            <div className="seat-list">{online.lobby.seats.map((seat, index) => <div className={seat.is_bot ? "bot-seat" : "human-seat"} key={seat.seat}><span>{index + 1}</span><div><strong>{seat.name}</strong><small>{seat.is_bot ? "AI fills this seat" : "Ready to play"}</small></div>{index === 0 && <Crown />}</div>)}</div>
            {online.lobby.host_user_id === online.user.id && online.lobby.requests.length > 0 && <div className="request-list"><strong>Join requests</strong>{online.lobby.requests.map(request => <div key={request.id}><span>{request.display_name}</span><Button size="sm" disabled={online.pending === `request:${request.id}`} onClick={() => onlineStore.respondJoin(request.id, true)}>{online.pending === `request:${request.id}` ? "Saving…" : "Accept"}</Button><Button size="sm" variant="ghost" disabled={online.pending === `request:${request.id}`} onClick={() => onlineStore.respondJoin(request.id, false)}>Decline</Button></div>)}</div>}
            <div className="lobby-actions">{online.lobby.host_user_id === online.user.id ? <Button size="lg" disabled={online.pending === "start"} onClick={() => onlineStore.startGame()}>{online.pending === "start" ? "Starting…" : "Start with this lineup"}</Button> : <Button variant="secondary" onClick={() => onlineStore.leaveLobby()}>Leave table</Button>}</div>
          </div> : <div className="table-browser">
            <div className="browser-heading"><div><strong>Open tables</strong><small>Ask the host for a seat</small></div><Button size="icon" variant="ghost" onClick={() => onlineStore.listLobbies()} aria-label="Refresh tables"><RefreshCw /></Button></div>
            <div className="table-list">{online.lobbies.length ? online.lobbies.map(lobby => <div className="table-row" key={lobby.id}>
              <div className="table-icon"><Crown /></div><div><strong>{lobby.name}</strong><small>Hosted by {lobby.host_name} · {lobby.rule_preset}</small></div>
              <span>{lobby.human_players}/4</span>
              {lobby.is_host ? <Button size="sm" disabled>Hosting</Button> : <Button size="sm" variant="secondary" disabled={lobby.requested || online.pending === `join:${lobby.id}`} onClick={() => onlineStore.requestJoin(lobby.id)}>{online.pending === `join:${lobby.id}` ? "Sending…" : lobby.requested ? "Requested" : "Request seat"}</Button>}
            </div>) : <div className="empty-tables"><Globe2 /><strong>No open tables yet</strong><span>Be the first host online.</span></div>}</div>
            <div className="create-table">
              <div className="section-label"><span><Plus /></span><div><strong>Host a new table</strong><small>Three AI players fill empty seats</small></div></div>
              <input value={tableName} onChange={e => setTableName(e.target.value)} placeholder={`${online.user.display_name}'s table`} maxLength={40} />
              <div className="create-options"><select value={preset} onChange={e => setPreset(e.target.value)}>{presets.map(p => <option value={p.id} key={p.id}>{p.name}</option>)}</select><select value={difficulty} onChange={e => setDifficulty(e.target.value)}><option value="easy">Easy AI</option><option value="medium">Medium AI</option><option value="hard">Hard AI</option></select></div>
              <Button disabled={online.pending === "create"} onClick={() => onlineStore.createLobby({ name: tableName, rule_preset: preset, bot_difficulty: difficulty })}>{online.pending === "create" ? "Creating table…" : "Create public table"}</Button>
            </div>
          </div>}
          {online.error && <div className="online-error" role="alert"><span>{online.error}</span><button onClick={() => onlineStore.clearError()} aria-label="Dismiss error">×</button></div>}
        </div>
      </section>
    </main>
  );
}

function GameHeader({ platform, onlineActive, onLocal }: {
  platform: "web" | "native"; onlineActive: boolean; onLocal: () => void;
}) {
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
            <button><Volume2 /> Sound effects <span>On</span></button>
            <button><Sparkles /> Motion effects <span>Full</span></button>
            <button><Shield /> Safe-cell hints <span>On</span></button>
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

  useEffect(() => {
    void gameStore.initialize();
    void onlineStore.restore();
  }, []);

  if (starting) return <LoadingScreen />;
  if (!model) {
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
      void gameStore.dispatch({
        NewGameWith: {
          preset: setup.preset,
          bot_difficulty: setup.botDifficulty
        }
      });
      setInGame(true);
    }} />;
  }

  const activeModel = online.model ?? model;
  const current = activeModel.players.find((player) => player.active);
  const status = activeModel.error ?? activeModel.status;
  const boardTokens = useMemo(
    () => activeModel.tokens.map((token) => ({
      ...token,
      selectable: token.selectable && (!online.model || current?.id === online.player)
    })),
    [activeModel.tokens, current?.id, online.model, online.player]
  );
  const handleTokenSelect = useCallback((token: number) => {
    if (online.model) {
      onlineStore.move(token);
      return;
    }
    void gameStore.dispatch({ SelectToken: token });
  }, [online.model]);
  const handleRoll = useCallback(() => {
    if (online.model) {
      onlineStore.roll();
      return;
    }
    void gameStore.dispatch("Roll");
  }, [online.model]);

  return (
    <main className="app-shell">
      <div className="ambient ambient-one" />
      <div className="ambient ambient-two" />
      <GameHeader platform={platform} onlineActive={Boolean(online.model)} onLocal={() => {
        onlineStore.showLocal();
        setInGame(false);
      }} />

      <section className="game-layout">
        <aside className="left-panel">
          <div className="panel-heading">
            <div><UsersRound /><span>Players</span></div>
            <small>Classic • 4 seats</small>
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
              <span>Shield marks protect your token from capture.</span>
            </div>
          </div>
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
          />
          <div className="mobile-status">{status}</div>
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
            disabled={!activeModel.can_roll || Boolean(online.model && current?.id !== online.player)}
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
          <div className="rule-note">
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
    </main>
  );
}
