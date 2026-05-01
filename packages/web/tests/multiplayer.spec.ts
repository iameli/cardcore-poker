import { test, expect, Page, Browser } from "@playwright/test";

/**
 * E2E multiplayer test: two browser contexts join one room, sit, ready up,
 * and play through the cryptographic dealing protocol until an action prompt
 * appears for whoever's turn it is. Validates the entire stack:
 *   browser → Svelte UI → PlayerSession → WasmAgent (Ristretto255) → WS relay
 *   → other browser → other WasmAgent → other Svelte UI.
 */

async function demoSignIn(page: Page) {
  await page.goto("/");
  await page.getByRole("button", { name: /Play in Demo Mode/i }).click();
  await expect(page.getByRole("heading", { name: /^Lobby$/i })).toBeVisible({
    timeout: 5000,
  });
}

async function joinRoom(page: Page, roomId: string) {
  await page.getByPlaceholder("Room ID").fill(roomId);
  await page
    .getByRole("button", { name: /^Join$/i })
    .first()
    .click();
  await expect(page.getByRole("heading", { name: /Waiting Room/i })).toBeVisible({
    timeout: 10_000,
  });
}

async function readyUp(page: Page) {
  // The server auto-seats joining players, so we just ready up.
  await expect(page.getByText("YOU")).toBeVisible({ timeout: 10_000 });
  await page.getByRole("button", { name: /Ready Up/i }).click();
}

async function expectGameStarted(page: Page) {
  // Waiting room disappears once gameState is created.
  await expect(page.getByRole("heading", { name: /Waiting Room/i })).toBeHidden({
    timeout: 30_000,
  });
  // Pot widget should be visible (shows "POT" inside the table).
  await expect(page.getByText("POT", { exact: true })).toBeVisible({
    timeout: 30_000,
  });
}

const ACTION_RX = /^(FOLD|CHECK|CALL|RAISE|ALL IN)$/;

async function waitUntilOnePlayerHasActions(p1: Page, p2: Page, timeoutMs = 60_000) {
  // Race: whichever player's turn it is should see an action button.
  const a1 = p1.getByRole("button", { name: ACTION_RX }).first();
  const a2 = p2.getByRole("button", { name: ACTION_RX }).first();
  const result = await Promise.race([
    a1.waitFor({ state: "visible", timeout: timeoutMs }).then(() => "p1"),
    a2.waitFor({ state: "visible", timeout: timeoutMs }).then(() => "p2"),
  ]);
  return result as "p1" | "p2";
}

async function freshContext(browser: Browser) {
  const ctx = await browser.newContext();
  const page = await ctx.newPage();
  return { ctx, page };
}

test.describe("multiplayer", () => {
  test("two players sit, ready, and the WASM dealing reaches a betting decision", async ({
    browser,
  }) => {
    const a = await freshContext(browser);
    const b = await freshContext(browser);

    for (const [tag, page] of [
      ["A", a.page],
      ["B", b.page],
    ] as const) {
      page.on("pageerror", (e) => console.log(`[${tag}-pageerror]`, e.message));
    }

    await demoSignIn(a.page);
    await demoSignIn(b.page);

    // Create the room via API so we know its ID; both players join it.
    const roomId: string = await a.page.evaluate(async () => {
      const res = await fetch("/api/rooms", { method: "POST" });
      const json = await res.json();
      return json.roomId;
    });
    expect(roomId).toMatch(/^[a-z0-9]+$/i);

    await joinRoom(a.page, roomId);
    await joinRoom(b.page, roomId);

    // Room-ID copy button shows the same id we created.
    await expect(a.page.getByTestId("copy-room-id")).toContainText(roomId);

    await readyUp(a.page);
    await readyUp(b.page);

    await expectGameStarted(a.page);
    await expectGameStarted(b.page);

    const acted = await waitUntilOnePlayerHasActions(a.page, b.page);
    console.log(`Acting player: ${acted}`);

    // The acting player's bet panel should include a RAISE button (preflop SB).
    const actingPage = acted === "p1" ? a.page : b.page;
    await expect(actingPage.getByRole("button", { name: /^RAISE$/ })).toBeVisible();

    // Fold and verify the hand ends.
    await actingPage.getByRole("button", { name: /^FOLD$/ }).click();

    // After fold, the dealer (seat 1 = page A) should see "DEAL NEW HAND".
    await expect(a.page.getByRole("button", { name: /DEAL NEW HAND/i })).toBeVisible({
      timeout: 30_000,
    });

    await a.ctx.close();
    await b.ctx.close();
  });
});
