import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import {
  Dialog, DialogContent, DialogDescription, DialogTitle, DialogTrigger
} from "./dialog";

function Example({ onOpenChange = vi.fn() }: { onOpenChange?: (open: boolean) => void }) {
  return (
    <Dialog onOpenChange={onOpenChange}>
      <DialogTrigger>Open profile</DialogTrigger>
      <DialogContent>
        <DialogTitle>Player profile</DialogTitle>
        <DialogDescription>Progress and match history.</DialogDescription>
      </DialogContent>
    </Dialog>
  );
}

describe("DialogContent", () => {
  it("opens with an accessible name and description", async () => {
    render(<Example />);
    await userEvent.click(screen.getByRole("button", { name: "Open profile" }));

    expect(screen.getByRole("dialog", { name: "Player profile" })).toBeVisible();
    expect(screen.getByText("Progress and match history.")).toBeVisible();
    expect(screen.getByRole("button", { name: "Close" })).toBeVisible();
  });

  it("closes from the explicit close control and Escape", async () => {
    const onOpenChange = vi.fn();
    render(<Example onOpenChange={onOpenChange} />);
    await userEvent.click(screen.getByRole("button", { name: "Open profile" }));
    await userEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(onOpenChange).toHaveBeenLastCalledWith(false);

    await userEvent.click(screen.getByRole("button", { name: "Open profile" }));
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });
});
