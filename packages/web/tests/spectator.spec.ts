import { test, expect } from "@playwright/test";
import { demoSignIn, freshContext, startOpenRoomGame } from "./helpers";

/**
 * E2E spectator test: two demo players start a game; a third demo account —
 * not in the roster — opens the game's /at:// URL and watches. The spectator
 * replays the whole game from the players' PDS records: it follows the
 * protocol phases, never gets action buttons, and sees the showdown.
 */

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

    // A and B start a game.
    const tableUri = await startOpenRoomGame(a, b);

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
