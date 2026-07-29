import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { PlayerViewModel } from "../../game/types";
import { Dice } from "./dice";
import { PlayerCard } from "./player-card";

const human: PlayerViewModel = {
  id: 0, name: "Alexandria", color: "Red", human: true, active: false, finished: 0
};

describe("Dice", () => {
  it.each([1, 2, 3, 4, 5, 6])("renders %i pips and an accessible value", value => {
    const { container } = render(<Dice value={value} busy={false} />);
    expect(screen.getByRole("img", { name: `Dice shows ${value}` })).toBeVisible();
    expect(container.querySelectorAll(".pip")).toHaveLength(value);
    expect(container.querySelector(".dice-crown")).not.toBeInTheDocument();
  });

  it("announces rolling and ready states without stale pips", () => {
    const { container, rerender } = render(<Dice value={null} busy />);
    expect(screen.getByRole("img", { name: "Dice rolling" })).toHaveClass("is-rolling");
    expect(container.querySelectorAll(".pip")).toHaveLength(0);
    expect(container.querySelector(".dice-crown")).toBeInTheDocument();

    rerender(<Dice value={null} busy={false} />);
    expect(screen.getByRole("img", { name: "Dice ready" })).not.toHaveClass("is-rolling");
  });
});

describe("PlayerCard", () => {
  it("shows human presence and progress", () => {
    render(<PlayerCard player={{ ...human, finished: 2 }} presence="reconnecting" />);
    expect(screen.getByText("Alexandria")).toBeVisible();
    expect(screen.getByText("Reconnecting")).toBeVisible();
    expect(screen.getByText("2")).toBeVisible();
    expect(screen.getByText("/4")).toBeVisible();
  });

  it("distinguishes active humans and bots", () => {
    const { rerender, container } = render(
      <PlayerCard player={{ ...human, active: true }} />
    );
    expect(screen.getByText("Playing now")).toBeVisible();
    expect(container.querySelector(".turn-pulse")).toBeInTheDocument();

    rerender(<PlayerCard player={{
      ...human, id: 1, name: "Royal AI", color: "Green", human: false, active: true
    }} />);
    expect(screen.getByText("AI is playing")).toBeVisible();
  });

  it("marks a completed player with a crown", () => {
    const { container } = render(<PlayerCard player={{ ...human, finished: 4 }} compact />);
    expect(container.querySelector(".is-compact")).toBeInTheDocument();
    expect(container.querySelector("svg.lucide-crown")).toBeInTheDocument();
  });
});
