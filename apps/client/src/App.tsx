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

function GameHeader({ platform }: { platform: "web" | "native" }) {
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
        <button className="active"><Gamepad2 /> Local</button>
        <button disabled title="Online play is planned"><Globe2 /> Online</button>
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
  const [confirmNew, setConfirmNew] = useState(false);

  useEffect(() => {
    void gameStore.initialize();
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

  const current = model.players.find((player) => player.active);
  const status = model.error ?? model.status;

  return (
    <main className="app-shell">
      <div className="ambient ambient-one" />
      <div className="ambient ambient-two" />
      <GameHeader platform={platform} />

      <section className="game-layout">
        <aside className="left-panel">
          <div className="panel-heading">
            <div><UsersRound /><span>Players</span></div>
            <small>Classic • 4 seats</small>
          </div>
          <div className="player-list">
            {model.players.map((player) => (
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
            {model.players.map((player) => (
              <PlayerCard key={player.id} player={player} compact />
            ))}
          </div>
          <LudoBoard
            tokens={model.tokens}
            onSelect={(token) => void gameStore.dispatch({ SelectToken: token })}
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
            <Dice value={model.dice} busy={model.busy} />
          </div>
          <Button
            size="lg"
            className="roll-button"
            disabled={!model.can_roll}
            onClick={() => void gameStore.dispatch("Roll")}
          >
            {model.busy
              ? <><Bot className="size-5" /> AI thinking…</>
              : model.can_roll
                ? "Roll dice"
                : model.human_turn
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
