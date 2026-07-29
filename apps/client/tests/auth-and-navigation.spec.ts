import { expect, test } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.goto("/");
  await page.evaluate(() => localStorage.clear());
  await page.reload();
});

test("authentication dialog supports login, registration, validation, and server errors", async ({ page }) => {
  await page.route("http://127.0.0.1:8080/api/auth/register", route =>
    route.fulfill({
      status: 409,
      contentType: "application/json",
      body: JSON.stringify({ error: "That email already has an account" })
    })
  );

  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page.getByRole("dialog", { name: "Login for online play" })).toBeVisible();
  await page.getByRole("button", { name: "Create a new account" }).click();
  await expect(page.getByRole("dialog", { name: "Create your player account" })).toBeVisible();

  await page.getByLabel("Display name").fill("Royal Tester");
  await page.getByLabel("Email").fill("royal@example.com");
  await page.getByLabel("Password").fill("correct-horse-battery-staple");
  await page.getByRole("button", { name: "Create account" }).click();
  await expect(page.getByText("That email already has an account", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Close" })).toBeVisible();
});

test("setup can be operated by keyboard and exposes visible focus", async ({ page }) => {
  const onlineTab = page.getByRole("button", { name: "Online tables", exact: true });
  await onlineTab.focus();
  await expect(onlineTab).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("heading", { name: "Sign in to join the tables" })).toBeVisible();
});

test("invalid saved games are discarded without blocking a new game", async ({ page }) => {
  await page.evaluate(() => localStorage.setItem("ludo-local-game-v1", "{not-json"));
  await page.reload();
  await expect(page.getByRole("button", { name: "Resume saved game" })).not.toBeVisible();
  await page.getByRole("button", { name: "Start solo game" }).click();
  await expect(page.getByRole("button", { name: "Roll dice" })).toBeVisible();
  await expect.poll(() => page.evaluate(() =>
    localStorage.getItem("ludo-local-game-v1")
  )).toBeNull();
});
