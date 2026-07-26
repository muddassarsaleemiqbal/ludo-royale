import { useEffect, useState } from "react";
import {
  Bot,
  Gamepad2,
  Globe2,
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
      <DialogTitle>Online play</DialogTitle>
      <DialogDescription>Signed in as {online.user.display_name}</DialogDescription>
      <div className="online-lobby">
        {online.model ? <p>Your match is ready. Close this window to play.</p>
          : online.queued ? <p>Looking for another player…</p>
            : <p>Play a secure, server-authoritative two-player match.</p>}
        {!online.model && (online.queued
          ? <Button variant="secondary" onClick={() => onlineStore.leaveQueue()}>Cancel search</Button>
          : <Button disabled={!online.connected} onClick={() => onlineStore.findMatch()}>Find match</Button>)}
        <Button variant="ghost" onClick={() => onlineStore.logout()}>Log out</Button>
      </div>
    </DialogContent>
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

  console.log("WELCOME TO LUDO ROYALE  ")

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

  const activeModel = online.model ?? model;
  const current = activeModel.players.find((player) => player.active);
  const status = activeModel.error ?? activeModel.status;

  return (
    <main className="app-shell">
      <div className="ambient ambient-one" />
      <div className="ambient ambient-two" />
      <GameHeader platform={platform} onlineActive={Boolean(online.model)} onLocal={() => onlineStore.showLocal()} />

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
            tokens={activeModel.tokens.map((token) => ({
              ...token,
              selectable: token.selectable && (!online.model || current?.id === online.player)
            }))}
            onSelect={(token) => online.model ? onlineStore.move(token) : void gameStore.dispatch({ SelectToken: token })}
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
            onClick={() => online.model ? onlineStore.roll() : void gameStore.dispatch("Roll")}
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
                  void gameStore.dispatch("NewGame");
                }}>Start new game</Button>
              </div>
            </DialogContent>
          </Dialog>
        </aside>
      </section>
    </main>
  );
}
