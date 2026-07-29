import { Bot, Crown, UserRound } from "lucide-react";
import { cn } from "../../lib/cn";
import type { PlayerViewModel } from "../../game/types";

export function PlayerCard({
  player,
  compact = false,
  presence
}: {
  player: PlayerViewModel;
  compact?: boolean;
  presence?: "online" | "reconnecting" | "offline";
}) {
  const bot = !player.human;
  return (
    <article
      className={cn(
        "player-card",
        `player-${player.color.toLowerCase()}`,
        player.active && "is-active",
        compact && "is-compact"
      )}
    >
      <div className="player-avatar">
        {bot ? <Bot /> : <UserRound />}
        {player.active && <span className="turn-pulse" />}
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5">
          <strong className="truncate">{player.name}</strong>
          {player.finished === 4 && <Crown className="size-3.5 text-amber-300" />}
        </div>
        <span>{player.active ? (bot ? "AI is playing" : "Playing now") : bot ? "AI player" : presence ? `${presence.charAt(0).toUpperCase()}${presence.slice(1)}` : "Human player"}</span>
      </div>
      <div className="home-count">
        <b>{player.finished}</b>
        <small>/4</small>
      </div>
    </article>
  );
}
