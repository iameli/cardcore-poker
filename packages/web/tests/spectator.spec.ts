import { test, expect, Page, Browser } from "@playwright/test";

/**
 * E2E spectator test: two demo players start a game; a third demo account —
 * not in the roster — opens the game's /at:// URL and watches. The spectator
 * replays the whole game from the players' PDS records: it follows the
 * protocol phases, never gets action buttons, and sees the showdown.
 */

async function demoSignIn(page: Page) {
  await page.goto("/");
  await page.getByRole("button", { name: /Play in Demo Mode/i }).click();
  await expect(page.getByRole("heading", { name: /^Lobby$/i })).toBeVisible({
    timeout: 15_000,
  });
}

async function readHandle(page: Page): Promise<string> {
  const text = await page.locator(".name").first().innerText();
  return text.trim();
}

async function freshContext(browser: Browser) {
  const ctx = await browser.newContext();
  await ctx.addInitScript(() => {
    try {
      localStorage.setItem("cardcore_unlocked", "1");
    } catch {}
  });
  const page = await ctx.newPage();
  return { ctx, page };
}

const ACTION_RX = /^(FOLD|CHECK|CALL|RAISE|ALL IN)$/;

test.describe("spectator (PDS-only)", () => {
  test("a non-player can watch a game via its URL", async ({ browser }) => {
    const a = await freshContext(browser);
    const b = await freshContext(browser);
    const c = await freshContext(browser); // spectator

    for (const [tag, page] of [
      ["A", a.page],
      ["B", b.page],
      ["C", c.page],
    ] as const) {
      page.on("pageerror", (e) => console.log(`[${tag}-pageerror]`, e.message));
      page.on("console", (m) => {
        if (m.type() === "error") console.log(`[${tag}-error]`, m.text());
      });
    }

    await Promise.all([demoSignIn(a.page), demoSignIn(b.page), demoSignIn(c.page)]);

    // A and B start a game via the invite flow.
    const handleB = await readHandle(b.page);
    await a.page.getByTestId("opponent-handle").fill(handleB);
    await a.page.getByTestId("create-table").click();
    await expect(a.page.getByTestId("copy-table-uri")).toBeVisible({ timeout: 15_000 });
    const tid = (await a.page.getByTestId("copy-table-uri").locator("code").innerText())
      .trim()
      .split("/")
      .pop()!;
    const didA = await a.page.evaluate(
      () => JSON.parse(localStorage.getItem("cardcore_demo_session")!).did,
    );
    const tableUri = `at://${didA}/re.cardco.poker.table/${tid.trim()}`;
    await b.page.getByTestId("join-uri").fill(tableUri);
    await b.page.getByTestId("join-table").click();
    await expect(b.page.getByTestId("copy-table-uri")).toBeVisible({ timeout: 15_000 });

    // C opens the game's URL and lands in the GameRoom as a spectator.
    await c.page.goto(`/${tableUri}`);
    await expect(c.page.getByTestId("spectating")).toBeVisible({ timeout: 30_000 });
    await expect(c.page.getByTestId("phase")).toBeVisible();

    // The spectator's log fills with replayed protocol actions.
    await expect(
      c.page
        .locator(".log-entry")
        .filter({ hasText: /commitSeed/ })
        .first(),
    ).toBeVisible({ timeout: 30_000 });

    // A player acts; the spectator never gets action buttons.
    const a1 = a.page.getByRole("button", { name: ACTION_RX }).first();
    const b1 = b.page.getByRole("button", { name: ACTION_RX }).first();
    const acted = await Promise.race([
      a1.waitFor({ state: "visible", timeout: 60_000 }).then(() => "A" as const),
      b1.waitFor({ state: "visible", timeout: 60_000 }).then(() => "B" as const),
    ]);
    const actingPage = acted === "A" ? a.page : b.page;
    await expect(c.page.getByRole("button", { name: ACTION_RX })).toHaveCount(0);

    // Fold; everyone — including the spectator — reaches showdown.
    await actingPage.getByRole("button", { name: /^FOLD$/ }).click();
    for (const page of [a.page, b.page, c.page]) {
      await expect(page.getByTestId("phase")).toHaveText(/showdown/, { timeout: 30_000 });
    }

    await a.ctx.close();
    await b.ctx.close();
    await c.ctx.close();
  });
});
