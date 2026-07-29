import { expect, test, type Page } from "@playwright/test";

const user = { id: "user-1", email: "royal@example.com", display_name: "Alexandria Royal" };
const hub = {
  profile: {
    user_id: user.id, display_name: user.display_name, xp: 1_275, level: 7,
    matches: 48, wins: 29, current_streak: 2, best_streak: 8, rating: 1_426,
    selected_dice: "emerald", selected_tokens: "marble"
  },
  friends: [
    { user_id: "friend-1", display_name: "A Very Long Player Name", level: 12, rating: 1_588, relationship: "friend", presence: "online" },
    { user_id: "friend-2", display_name: "Morgan", level: 5, rating: 1_201, relationship: "incoming", presence: "offline" }
  ],
  matches: [
    { id: "match-1", played_at: "2026-07-29T12:00:00Z", placement: 1, xp_earned: 125, rating_delta: 18, ranked: true, opponents: ["Morgan", "A Very Long Opponent Name", "Royal AI"] },
    { id: "match-2", played_at: "2026-07-28T12:00:00Z", placement: 3, xp_earned: 45, rating_delta: -9, ranked: false, opponents: ["Kai", "Sam", "Royal AI"] }
  ],
  achievements: ["first_win", "veteran_10", "streak_3"],
  challenges: [
    { key: "play", title: "Play two matches", progress: 1, target: 2, reward: 100, claimed: false },
    { key: "capture", title: "Capture three rival tokens", progress: 3, target: 3, reward: 150, claimed: true }
  ],
  leaderboard: Array.from({ length: 8 }, (_, index) => ({
    rank: index + 1, user_id: index === 3 ? user.id : `leader-${index}`,
    display_name: index === 3 ? user.display_name : `Season Player ${index + 1}`,
    rating: 1_700 - index * 42, matches: 30 + index, wins: 21 - index
  })),
  season_name: "Golden Crown Season", season_ends_at: "2026-09-01T00:00:00Z",
  invites: [{ id: "invite-1", lobby_id: "lobby-2", lobby_name: "Friday Royals", sender_name: "Morgan" }]
};
const lobbies = [
  { id: "lobby-1", name: "Weekend Championship Warmup", host_name: "A Very Long Host Name", human_players: 3, rule_preset: "tournament", bot_difficulty: "hard", status: "waiting", is_host: false, requested: false },
  { id: "lobby-2", name: "Fast Friends", host_name: "Morgan", human_players: 2, rule_preset: "quick", bot_difficulty: "medium", status: "playing", is_host: false, requested: false }
];
const lobby = {
  id: "lobby-1", name: "Alexandria's Championship Table", host_user_id: user.id,
  rule_preset: "tournament", bot_difficulty: "hard", status: "waiting",
  invite_code: "ROYAL123", is_public: false, turn_seconds: 30, spectator_count: 2,
  ranked: false, rematch_mode: "vote",
  seats: [
    { seat: 0, user_id: user.id, name: user.display_name, is_bot: false, ready: true, presence: "online" },
    { seat: 1, user_id: "friend-1", name: "A Very Long Player Name", is_bot: false, ready: false, presence: "reconnecting" },
    { seat: 2, user_id: null, name: "Royal AI", is_bot: true, ready: true, presence: "bot" },
    { seat: 3, user_id: null, name: "Royal AI", is_bot: true, ready: true, presence: "bot" }
  ],
  requests: [{ id: "request-1", user_id: "requester-1", display_name: "Player Request With Long Name" }]
};

async function mockOnline(page: Page) {
  await page.route("http://127.0.0.1:8080/api/me", route =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(user) })
  );
  await page.route("http://127.0.0.1:8080/api/ably/token", route =>
    route.fulfill({ status: 503, body: "disabled in responsive test" })
  );
  await page.addInitScript(({ mockHub, mockLobbies, mockLobby }) => {
    localStorage.setItem("ludo-online-token", "responsive-test-token");
    class MockWebSocket {
      static readonly CONNECTING = 0;
      static readonly OPEN = 1;
      static readonly CLOSING = 2;
      static readonly CLOSED = 3;
      readyState = MockWebSocket.CONNECTING;
      onopen: (() => void) | null = null;
      onclose: (() => void) | null = null;
      onerror: (() => void) | null = null;
      onmessage: ((event: { data: string }) => void) | null = null;
      constructor() {
        setTimeout(() => {
          this.readyState = MockWebSocket.OPEN;
          this.onopen?.();
          this.onmessage?.({ data: JSON.stringify({ type: "lobby_list", lobbies: mockLobbies }) });
          this.onmessage?.({ data: JSON.stringify({ type: "hub", hub: mockHub }) });
        });
      }
      send(raw: string) {
        const message = JSON.parse(raw) as { type: string; command_id: string };
        const reply = (value: unknown) => setTimeout(() =>
          this.onmessage?.({ data: JSON.stringify(value) })
        );
        if (message.type === "list_lobbies") reply({ type: "lobby_list", lobbies: mockLobbies });
        if (message.type === "get_hub") reply({ type: "hub", hub: mockHub });
        if (message.type === "create_lobby") reply({ type: "lobby", lobby: mockLobby });
        reply({ type: "ack", command_id: message.command_id });
      }
      close() {
        this.readyState = MockWebSocket.CLOSED;
        this.onclose?.();
      }
    }
    Object.assign(MockWebSocket, { CONNECTING: 0, OPEN: 1, CLOSING: 2, CLOSED: 3 });
    Object.defineProperty(window, "WebSocket", { value: MockWebSocket });
  }, { mockHub: hub, mockLobbies: lobbies, mockLobby: lobby });
}

async function expectResponsive(page: Page) {
  await expect.poll(() => page.evaluate(() =>
    document.documentElement.scrollWidth <= window.innerWidth
  )).toBe(true);
}

test("online browser and player hub remain usable across screen sizes", async ({ page }) => {
  await mockOnline(page);
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
    await expect(page.getByRole("button", { name: "Online tables" })).toBeVisible();
    await page.getByRole("button", { name: "Online tables" }).click();
    await expect(page.getByText("Weekend Championship Warmup")).toBeVisible();
    await expectResponsive(page);

    await page.getByRole("button", { name: /Alexandria Royal/ }).click();
    await expect(page.getByRole("dialog")).toBeVisible();
    await expect(page.getByRole("button", { name: "Close" })).toBeVisible();
    await expectResponsive(page);
    for (const tab of ["Friends", "Matches", "Ranked", "Rewards", "Profile"]) {
      await page.getByRole("button", { name: tab, exact: true }).click();
      await expectResponsive(page);
    }
    await page.keyboard.press("Escape");
  }
});

test("host lobby controls stack without overflow on compact screens", async ({ page }) => {
  await mockOnline(page);
  for (const viewport of [{ width: 320, height: 568 }, { width: 844, height: 390 }]) {
    await page.setViewportSize(viewport);
    await page.goto("/");
    await page.getByRole("button", { name: "Online tables" }).click();
    await page.getByLabel("Table name").fill("Responsive test");
    await page.getByRole("button", { name: "Create public table" }).click();
    await expect(page.getByText("Alexandria's Championship Table")).toBeVisible();
    await expect(page.getByLabel("Lobby rules")).toBeVisible();
    await expectResponsive(page);
  }
});

test("table filters and invite entry remain functional with realistic data", async ({ page }) => {
  await mockOnline(page);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/");
  await page.getByRole("button", { name: "Online tables" }).click();

  await page.getByLabel("Filter by rules").selectOption("quick");
  await expect(page.getByText("Fast Friends")).toBeVisible();
  await expect(page.getByText("Weekend Championship Warmup")).not.toBeVisible();

  await page.getByLabel("Filter by rules").selectOption("all");
  await page.getByLabel("Filter by occupancy").selectOption("nearly");
  await expect(page.getByText("Weekend Championship Warmup")).toBeVisible();
  await expect(page.getByText("Fast Friends")).not.toBeVisible();

  await page.getByLabel("Invite code").fill("royal123");
  await expect(page.getByLabel("Invite code")).toHaveValue("ROYAL123");
  await page.getByRole("button", { name: "Join invite" }).click();
  await expectResponsive(page);
});

test("player hub tabs expose their expected content and close with Escape", async ({ page }) => {
  await mockOnline(page);
  await page.goto("/");
  await page.getByRole("button", { name: /Alexandria Royal/ }).click();

  await page.getByRole("button", { name: "Friends", exact: true }).click();
  await expect(page.getByText("A Very Long Player Name")).toBeVisible();
  await expect(page.getByText("Morgan invited you")).toBeVisible();

  await page.getByRole("button", { name: "Matches", exact: true }).click();
  await expect(page.getByText("Ranked match")).toBeVisible();
  await expect(page.getByText("+125 XP · +18")).toBeVisible();

  await page.getByRole("button", { name: "Ranked", exact: true }).click();
  await expect(page.getByText("Golden Crown Season")).toBeVisible();
  await expect(page.getByText("Season Player 1")).toBeVisible();

  await page.getByRole("button", { name: "Rewards", exact: true }).click();
  await expect(page.getByText("Daily challenges")).toBeVisible();
  await expect(page.getByText("First Crown")).toBeVisible();

  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).not.toBeVisible();
});
