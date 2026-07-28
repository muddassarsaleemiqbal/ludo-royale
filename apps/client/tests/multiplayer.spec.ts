import { expect, test, type Page } from "@playwright/test";

async function register(page: Page, identity: string) {
  await page.goto("/");
  await page.getByRole("button", { name: "Online tables" }).click();
  await page.getByRole("button", { name: "Sign in or create account" }).click();
  await page.getByRole("button", { name: "Create a new account" }).click();
  await page.getByLabel("Display name").fill(identity);
  await page.getByLabel("Email").fill(`${identity.toLowerCase()}@example.test`);
  await page.getByLabel("Password").fill("correct-horse-battery-staple");
  await page.getByRole("button", { name: "Create account" }).click();
  await expect(page.locator(".setup-profile")).toHaveText(identity, {
    timeout: 15_000
  });
}

test("two players can join, start, act, and reconnect", async ({ browser }) => {
  const suffix = `${Date.now()}-${Math.floor(Math.random() * 10_000)}`;
  const hostContext = await browser.newContext();
  const guestContext = await browser.newContext();
  const host = await hostContext.newPage();
  const guest = await guestContext.newPage();
  await register(host, `Host-${suffix}`);
  await register(guest, `Guest-${suffix}`);

  await host.getByLabel("Table name").fill(`Royal-${suffix}`);
  await host.getByRole("button", { name: "Create public table" }).click();
  await expect(host.getByText(`Royal-${suffix}`)).toBeVisible();

  await guest.getByRole("button", { name: "Refresh tables" }).click();
  const table = guest.locator(".table-row").filter({ hasText: `Royal-${suffix}` });
  await expect(table).toHaveCount(1);
  await table.getByRole("button", { name: "Request seat" }).click();
  await expect(guest.getByText("Join request sent to the host.")).toBeVisible();

  await expect(host.getByText(`Guest-${suffix}`)).toBeVisible();
  await host.getByRole("button", { name: "Accept" }).click();
  await expect(guest.getByText("Your seat was accepted.")).toBeVisible();
  await guest.getByRole("button", { name: "I'm ready" }).click();
  await expect(guest.getByRole("button", { name: "Not ready" })).toBeVisible();
  await host.getByRole("button", { name: "Start with this lineup" }).click();
  await expect(host.getByRole("button", { name: "Roll dice" })).toBeVisible();

  await host.reload();
  await expect(host.locator(".turn-banner")).toContainText("Your turn");
  await host.getByRole("button", { name: "Roll dice" }).click();
  await expect(host.locator(".event-feed")).toContainText("rolled the dice");

  await hostContext.close();
  await guestContext.close();
});
