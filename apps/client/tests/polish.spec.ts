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

test("local progress survives a reload and can be resumed", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Start solo game" }).click();
  await page.getByRole("button", { name: "Roll dice" }).click();
  await expect.poll(() => page.evaluate(() =>
    Boolean(localStorage.getItem("ludo-local-game-v1"))
  )).toBe(true);

  const revision = await page.evaluate(() => {
    const value = localStorage.getItem("ludo-local-game-v1");
    return value ? (JSON.parse(value) as { revision: number }).revision : -1;
  });
  await page.reload();
  await expect(page.getByRole("button", { name: "Resume saved game" })).toBeVisible();
  await page.getByRole("button", { name: "Resume saved game" }).click();
  await expect(page.locator(".turn-banner")).toBeVisible();
  await expect.poll(() => page.evaluate(() => {
    const value = localStorage.getItem("ludo-local-game-v1");
    return value ? (JSON.parse(value) as { revision: number }).revision : -1;
  })).toBe(revision);
});

test("game remains usable without horizontal overflow across screen sizes", async ({ page }) => {
  const viewports = [
    { width: 320, height: 568 },
    { width: 390, height: 844 },
    { width: 768, height: 1024 },
    { width: 844, height: 390 },
    { width: 1024, height: 768 },
    { width: 1440, height: 900 }
  ];

  for (const viewport of viewports) {
    await page.setViewportSize(viewport);
    await page.goto("/");
    await page.evaluate(() => localStorage.clear());
    await page.reload();
    await page.getByRole("button", { name: "Start solo game" }).click();
    await expect(page.getByRole("button", { name: "Roll dice" })).toBeVisible();
    await expect.poll(() => page.evaluate(() =>
      document.documentElement.scrollWidth <= window.innerWidth
    )).toBe(true);
  }
});
