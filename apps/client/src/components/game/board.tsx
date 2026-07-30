import { memo, useEffect, useMemo, useRef, useState } from "react";
import { Flag, ShieldCheck } from "lucide-react";
import { cn } from "../../lib/cn";
import type {
  PlayerColor,
  TokenPosition,
  TokenViewModel
} from "../../game/types";

const TRACK: ReadonlyArray<readonly [number, number]> = [
  [6,1],[6,2],[6,3],[6,4],[6,5],[5,6],[4,6],[3,6],[2,6],[1,6],[0,6],[0,7],[0,8],
  [1,8],[2,8],[3,8],[4,8],[5,8],[6,9],[6,10],[6,11],[6,12],[6,13],[6,14],[7,14],
  [8,14],[8,13],[8,12],[8,11],[8,10],[8,9],[9,8],[10,8],[11,8],[12,8],[13,8],
  [14,8],[14,7],[14,6],[13,6],[12,6],[11,6],[10,6],[9,6],[8,5],[8,4],[8,3],
  [8,2],[8,1],[8,0],[7,0],[6,0]
];

const COLORS: PlayerColor[] = ["Red", "Green", "Yellow", "Blue"];
const STARTS = [0, 13, 26, 39];
const SAFE = new Set(["6:1", "1:8", "8:13", "13:6", "2:6", "6:12", "12:8", "8:2"]);

function coordinate(
  color: PlayerColor,
  token: number,
  position: TokenPosition
): readonly [number, number] | null {
  const colorIndex = COLORS.indexOf(color);
  if (position === "Finished") return null;
  if (position === "Yard") {
    const offsets = [[0,0],[0,2],[2,0],[2,2]] as const;
    const bases = [[2,2],[2,10],[10,10],[10,2]] as const;
    const base = bases[colorIndex] ?? bases[0];
    const offset = offsets[token] ?? offsets[0];
    return [base[0] + offset[0], base[1] + offset[1]];
  }
  const progress = position.Path;
  if (progress < 52) {
    return TRACK[((STARTS[colorIndex] ?? 0) + progress) % 52] ?? null;
  }
  const offset = progress - 52;
  return [
    [7, 1 + offset],
    [1 + offset, 7],
    [7, 13 - offset],
    [13 - offset, 7]
  ][colorIndex] as [number, number];
}

function cellKind(row: number, column: number) {
  if (row < 6 && column < 6) return "red-yard";
  if (row < 6 && column > 8) return "green-yard";
  if (row > 8 && column > 8) return "yellow-yard";
  if (row > 8 && column < 6) return "blue-yard";
  if ((row === 7 && column >= 1 && column <= 5) || (row === 6 && column === 1)) return "red-home";
  if ((column === 7 && row >= 1 && row <= 5) || (row === 1 && column === 8)) return "green-home";
  if ((row === 7 && column >= 9 && column <= 13) || (row === 8 && column === 13)) return "yellow-home";
  if ((column === 7 && row >= 9 && row <= 13) || (row === 13 && column === 6)) return "blue-home";
  if (row >= 6 && row <= 8 && column >= 6 && column <= 8) return "center";
  return "track";
}

const BoardToken = memo(function BoardToken({
  token,
  onSelect,
  isRecent,
  isCaptured,
  reachedHome,
  stackIndex,
  stackSize
}: {
  token: TokenViewModel;
  onSelect(token: number): void;
  isRecent: boolean;
  isCaptured: boolean;
  reachedHome: boolean;
  stackIndex: number;
  stackSize: number;
}) {
  const point = coordinate(token.color, token.token, token.position);
  const preview = token.preview ? coordinate(token.color, token.token, token.preview) : null;
  if (!point) return null;
  return (
    <>
      {token.selectable && preview && (
        <span
          className={`move-preview preview-${token.color.toLowerCase()}`}
          style={{
            "--token-row": preview[0],
            "--token-column": preview[1]
          } as React.CSSProperties}
          aria-hidden="true"
        >
          <Flag />
          <b>{token.token + 1}</b>
        </span>
      )}
      <button
        className={cn(
          "game-token",
          `token-${token.color.toLowerCase()}`,
          token.selectable && "is-selectable",
          isRecent && "is-recent",
          isCaptured && "is-captured",
          reachedHome && "reached-home"
        )}
        style={{
          "--token-row": point[0],
          "--token-column": point[1],
          "--stack-x": stackSize > 1 ? `${stackIndex % 2 ? 18 : -18}%` : "0%",
          "--stack-y": stackSize > 2 ? `${stackIndex > 1 ? 18 : -18}%` : "0%"
        } as React.CSSProperties}
        disabled={!token.selectable}
        onClick={() => onSelect(token.token)}
        aria-label={`Move ${token.color} token ${token.token + 1}${token.preview ? " to the highlighted square" : ""}`}
      >
        <span>{token.token + 1}</span>
      </button>
    </>
  );
});

export const LudoBoard = memo(function LudoBoard({
  tokens,
  onSelect,
  showSafeCells = true,
  animate = true,
  recentMoveKey = null,
  capturedKeys = [],
  homeKey = null
}: {
  tokens: TokenViewModel[];
  onSelect(token: number): void;
  showSafeCells?: boolean;
  animate?: boolean;
  recentMoveKey?: string | null;
  capturedKeys?: string[];
  homeKey?: string | null;
}) {
  const [displayPositions, setDisplayPositions] = useState<Record<string, TokenPosition>>({});
  const previousTokens = useRef(tokens);
  useEffect(() => {
    const timers: number[] = [];
    const previous = previousTokens.current;
    const nextPositions: Record<string, TokenPosition> = {};
    for (const token of tokens) {
      const key = `${token.player}:${token.token}`;
      const before = previous.find(item => item.player === token.player && item.token === token.token);
      if (
        animate &&
        before &&
        typeof before.position === "object" &&
        typeof token.position === "object" &&
        token.position.Path > before.position.Path
      ) {
        const distance = token.position.Path - before.position.Path;
        for (let step = 1; step <= distance; step += 1) {
          const position: TokenPosition = { Path: before.position.Path + step };
          timers.push(window.setTimeout(() => {
            setDisplayPositions(current => ({ ...current, [key]: position }));
          }, Math.min(step, 10) * 58));
        }
      } else {
        nextPositions[key] = token.position;
      }
    }
    setDisplayPositions(current => ({ ...current, ...nextPositions }));
    previousTokens.current = tokens;
    return () => timers.forEach(timer => window.clearTimeout(timer));
  }, [animate, tokens]);
  const displayTokens = useMemo(() => {
    const positioned = tokens.map(token => ({
      ...token,
      position: displayPositions[`${token.player}:${token.token}`] ?? token.position
    }));
    const groups = new Map<string, number[]>();
    positioned.forEach((token, index) => {
      const point = coordinate(token.color, token.token, token.position);
      if (point) groups.set(point.join(":"), [...(groups.get(point.join(":")) ?? []), index]);
    });
    return positioned.map((token, index) => {
      const point = coordinate(token.color, token.token, token.position);
      const group = point ? groups.get(point.join(":")) ?? [index] : [index];
      return { ...token, stackIndex: group.indexOf(index), stackSize: group.length };
    });
  }, [displayPositions, tokens]);
  const cells = useMemo(() => Array.from({ length: 225 }, (_, index) => {
    const row = Math.floor(index / 15);
    const column = index % 15;
    const safe = SAFE.has(`${row}:${column}`);
    return (
      <div
        className={cn("board-cell", cellKind(row, column))}
        key={index}
        aria-hidden="true"
      >
        {safe && showSafeCells && <ShieldCheck className="safe-mark" />}
      </div>
    );
  }), [showSafeCells]);

  return (
    <div className="board-frame">
      <div className="board-glow" />
      <div className="ludo-board" role="group" aria-label="Ludo board. Shield symbols mark safe cells.">
        {cells}
        <div className="home-crown" aria-hidden="true">♛</div>
        {displayTokens.map((token) => (
          <BoardToken
            key={`${token.player}-${token.token}`}
            token={token}
            onSelect={onSelect}
            isRecent={recentMoveKey === `${token.player}:${token.token}`}
            isCaptured={capturedKeys.includes(`${token.player}:${token.token}`)}
            reachedHome={homeKey === `${token.player}:${token.token}`}
            stackIndex={token.stackIndex}
            stackSize={token.stackSize}
          />
        ))}
      </div>
    </div>
  );
});
