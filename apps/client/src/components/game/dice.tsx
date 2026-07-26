import { cn } from "../../lib/cn";

const PIPS: Record<number, Array<[number, number]>> = {
  1: [[2, 2]],
  2: [[1, 1], [3, 3]],
  3: [[1, 1], [2, 2], [3, 3]],
  4: [[1, 1], [3, 1], [1, 3], [3, 3]],
  5: [[1, 1], [3, 1], [2, 2], [1, 3], [3, 3]],
  6: [[1, 1], [3, 1], [1, 2], [3, 2], [1, 3], [3, 3]]
};

export function Dice({ value, busy }: { value: number | null; busy: boolean }) {
  return (
    <div className={cn("dice", busy && "is-rolling")} aria-label={value ? `Dice shows ${value}` : "Dice"}>
      {(PIPS[value ?? 0] ?? []).map(([column, row], index) => (
        <span
          key={index}
          className="pip"
          style={{ gridColumn: column, gridRow: row }}
        />
      ))}
      {!value && <span className="dice-crown">♛</span>}
    </div>
  );
}
