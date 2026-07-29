import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { Button } from "./button";

describe("Button", () => {
  it("uses safe defaults and forwards click behavior", async () => {
    const onClick = vi.fn();
    render(<Button onClick={onClick}>Roll dice</Button>);
    const button = screen.getByRole("button", { name: "Roll dice" });
    expect(button).toHaveClass("h-11");
    expect(button).toHaveClass("from-amber-300");
    await userEvent.click(button);
    expect(onClick).toHaveBeenCalledOnce();
  });

  it("supports variants, sizes, custom classes, and disabled state", async () => {
    const onClick = vi.fn();
    render(
      <Button variant="danger" size="sm" className="test-class" disabled onClick={onClick}>
        Remove
      </Button>
    );
    const button = screen.getByRole("button", { name: "Remove" });
    expect(button).toHaveClass("bg-red-500/15", "h-9", "test-class");
    expect(button).toBeDisabled();
    await userEvent.click(button);
    expect(onClick).not.toHaveBeenCalled();
  });

  it("can style a child element without adding a nested button", () => {
    render(<Button asChild><a href="/rules">Rules</a></Button>);
    const link = screen.getByRole("link", { name: "Rules" });
    expect(link).toHaveAttribute("href", "/rules");
    expect(link).toHaveClass("rounded-xl");
  });
});
