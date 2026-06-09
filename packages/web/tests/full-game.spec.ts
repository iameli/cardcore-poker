import { test, expect, Page, Browser } from "@playwright/test";

/**
 * E2E multi-hand test: two demo players play a full game across several hands
 * until one player has all the chips.
 *
 * Validates the multi-hand loop end to end:
 *   - a hand reaches showdown and the result is written to the log,
 *   - the next hand starts automatically (no interaction),
 *   - going all-in busts a player and the game ends with a single winner.
 *
 * Strategy: play the first hand passively (check/call) so nobody busts — this
 * forces a continuation to a second hand — then go all-in to end the game fast.
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

async function clickIfVisible(page: Page, rx: RegExp): Promise<boolean> {
  const btn = page.getByRole("button", { name: rx }).first();
  if (await btn.isVisible().catch(() => false)) {
    await btn.click().catch(() => {});
    return true;
  }
  return false;
}

// Passive: check, else call. Aggressive: shove, else call, else check.
async function act(page: Page, aggressive: boolean) {
  if (aggressive) {
    if (await clickIfVisible(page, /^ALL IN$/)) return;
    if (await clickIfVisible(page, /^CALL$/)) return;
    if (await clickIfVisible(page, /^CHECK$/)) return;
  } else {
    if (await clickIfVisible(page, /^CHECK$/)) return;
    if (await clickIfVisible(page, /^CALL$/)) return;
  }
}

test.describe("full game (PDS-only)", () => {
  test("two players play multiple hands until one wins", async ({ browser }) => {
    test.setTimeout(180_000);

    const a = await freshContext(browser);
    const b = await freshContext(browser);

    for (const [tag, page] of [
      ["A", a.page],
      ["B", b.page],
    ] as const) {
      page.on("pageerror", (e) => console.log(`[${tag}-pageerror]`, e.message));
      page.on("console", (m) => {
        if (m.type() === "error") console.log(`[${tag}-error]`, m.text());
      });
    }

    await Promise.all([demoSignIn(a.page), demoSignIn(b.page)]);

    const handleB = await readHandle(b.page);
    await a.page.getByTestId("opponent-handle").fill(handleB);
    await a.page.getByTestId("create-table").click();

    // A is in the GameRoom; reconstruct the table URI and have B join.
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

    const pages = [a.page, b.page];
    let aggressive = false;
    let firstHandLogged = false;
    let sawGameOver = false;

    const deadline = Date.now() + 120_000;
    while (Date.now() < deadline) {
      if (
        (await a.page
          .getByTestId("game-over")
          .isVisible()
          .catch(() => false)) ||
        (await b.page
          .getByTestId("game-over")
          .isVisible()
          .catch(() => false))
      ) {
        sawGameOver = true;
        break;
      }

      // Once the first hand's result is in the log, the game has auto-advanced
      // to a second hand — switch to all-in to finish quickly.
      if (!aggressive) {
        const logged = await a.page
          .getByText(/results/i)
          .first()
          .isVisible()
          .catch(() => false);
        if (logged) {
          aggressive = true;
          firstHandLogged = true;
          // During the showdown pause, the opponent's revealed hole cards are
          // laid face-up on the table (not just described in the log).
          await expect(a.page.locator(".seat-area.top .card-face")).toHaveCount(2, {
            timeout: 3000,
          });
        }
      }

      for (const p of pages) await act(p, aggressive);
      await a.page.waitForTimeout(300);
    }

    // The first hand reached showdown and was logged (proves continuation),
    // and the game ended with one winner (proves the win condition).
    expect(firstHandLogged, "first hand result should be logged").toBe(true);
    expect(sawGameOver, "game should end with one winner").toBe(true);
    await expect(a.page.getByText(/wins/i).first()).toBeVisible();

    await a.ctx.close();
    await b.ctx.close();
  });
});
