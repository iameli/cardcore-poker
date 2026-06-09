import { test, expect, Page, Browser } from "@playwright/test";

/**
 * E2E open-room matchmaking test (host approves joiner).
 *
 * Validates the consensus join flow end to end:
 *   1. Host A creates an open room (table with just themselves).
 *   2. A lands on the host RoomLobby and shares the path-based room link.
 *   3. Joiner B opens the link, lands on the join screen, and requests to join
 *      (publishing a table record at the same rkey with themselves appended).
 *   4. A discovers B's request, approves it, then starts the hand (publishing
 *      the locked roster + startedAt as the canonical table record).
 *   5. B polls the host record, sees itself in the started roster, and both
 *      players enter the game and play one hand to showdown — over the real
 *      firehose, not the discovery stream.
 */

async function demoSignIn(page: Page) {
  await page.goto("/");
  await page.getByRole("button", { name: /Play in Demo Mode/i }).click();
  await expect(page.getByRole("heading", { name: /^Lobby$/i })).toBeVisible({
    timeout: 15_000,
  });
}

async function freshContext(browser: Browser) {
  const ctx = await browser.newContext();
  // Pre-unlock the soft-launch gate so each context skips the password screen.
  await ctx.addInitScript(() => {
    try {
      localStorage.setItem("cardcore_unlocked", "1");
    } catch {}
  });
  const page = await ctx.newPage();
  return { ctx, page };
}

const ACTION_RX = /^(FOLD|CHECK|CALL|RAISE|ALL IN)$/;

test.describe("open room (PDS-only)", () => {
  test("host opens a room, joiner requests, host approves & starts, hand plays", async ({
    browser,
  }) => {
    const a = await freshContext(browser); // host
    const b = await freshContext(browser); // joiner

    a.page.on("pageerror", (e) => console.log("[A-pageerror]", e.message));
    b.page.on("pageerror", (e) => console.log("[B-pageerror]", e.message));
    for (const [tag, page] of [
      ["A", a.page],
      ["B", b.page],
    ] as const) {
      page.on("console", (m) => {
        if (m.type() === "error" || m.type() === "warning") {
          console.log(`[${tag}-${m.type()}]`, m.text());
        }
      });
    }

    // Sign in two distinct demo accounts.
    await Promise.all([demoSignIn(a.page), demoSignIn(b.page)]);

    const didA = await a.page.evaluate(
      () => JSON.parse(localStorage.getItem("cardcore_demo_session")!).did,
    );

    // A creates an open room → host RoomLobby.
    await a.page.getByTestId("create-open-room").click();
    await expect(a.page.getByTestId("copy-table-uri")).toBeVisible({ timeout: 15_000 });

    // Reconstruct the room URI from A's DID + the shared tid.
    const tid = await a.page.getByTestId("copy-table-uri").locator("code").innerText();
    const tableUri = `at://${didA}/re.cardco.poker.table/${tid.trim()}`;
    console.log(`open room: ${tableUri}`);

    // B opens the path-based room link.
    await b.page.goto(`/${tableUri}`);
    await expect(b.page.getByTestId("request-join")).toBeVisible({ timeout: 15_000 });

    // B requests to join (confirm step).
    await b.page.getByTestId("request-join").click();
    await expect(b.page.getByTestId("join-status")).toBeVisible({ timeout: 15_000 });

    // A discovers the request and approves it.
    await expect(a.page.getByTestId("approve-request")).toBeVisible({ timeout: 30_000 });
    await a.page.getByTestId("approve-request").click();

    // A starts the hand (locks the roster).
    await expect(a.page.getByTestId("start-game")).toBeEnabled();
    await a.page.getByTestId("start-game").click();

    // Both players land in the GameRoom (phase label is GameRoom-only).
    await expect(a.page.getByTestId("phase")).toBeVisible({ timeout: 30_000 });
    await expect(b.page.getByTestId("phase")).toBeVisible({ timeout: 30_000 });

    // The table URL persists into (and through) the game on both sides.
    await expect(a.page).toHaveURL(/\/at:\/\//);
    await expect(b.page).toHaveURL(/\/at:\/\//);

    // The cryptographic deal runs over the firehose; one side gets to act.
    const a1 = a.page.getByRole("button", { name: ACTION_RX }).first();
    const b1 = b.page.getByRole("button", { name: ACTION_RX }).first();
    const acted = await Promise.race([
      a1.waitFor({ state: "visible", timeout: 60_000 }).then(() => "A" as const),
      b1.waitFor({ state: "visible", timeout: 60_000 }).then(() => "B" as const),
    ]);
    console.log(`Acting first: ${acted}`);

    const actingPage = acted === "A" ? a.page : b.page;
    await expect(actingPage.getByRole("button", { name: /^RAISE$/ })).toBeVisible();

    // Fold and verify the hand reaches showdown on both sides.
    await actingPage.getByRole("button", { name: /^FOLD$/ }).click();
    for (const page of [a.page, b.page]) {
      await expect(page.getByTestId("phase")).toHaveText(/showdown/, { timeout: 30_000 });
    }

    await a.ctx.close();
    await b.ctx.close();
  });
});
