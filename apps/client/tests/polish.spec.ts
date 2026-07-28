import { expect, test } from "@playwright/test";

test("solo setup, game status, preferences, and accessibility controls work", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Set the rules. Claim the crown." })).toBeVisible();
  await page.getByRole("button", { name: "Start solo game" }).click();
  await expect(page.locator(".turn-banner")).toHaveText("Your turn");
  await expect(page.getByRole("button", { name: "Roll dice" })).toBeVisible();

  await page.getByRole("button", { name: "Settings" }).click();
  const sound = page.getByRole("switch", { name: "Sound effects On" });
  await expect(sound).toHaveAttribute("aria-checked", "true");
  await sound.click();
  await expect(page.getByRole("switch", { name: "Sound effects Muted" })).toHaveAttribute(
    "aria-checked",
    "false"
  );

  const motion = page.getByRole("switch", { name: "Motion effects Full" });
  await motion.click();
  await expect(page.locator("html")).toHaveAttribute("data-motion", "reduced");
  await page.keyboard.press("Escape");

  await page.getByRole("button", { name: "Roll dice" }).click();
  await expect(page.getByRole("img", { name: /Dice/ })).toBeVisible();
});
