import { test, expect } from "@playwright/test";
import { demoSignIn, freshContext, startOpenRoomGame } from "./helpers";

const ACTION_RX = /^(FOLD|CHECK|CALL|RAISE|ALL IN)$/;

/**
 * The previous hand's result floats over the table for ~7s while the next
 * hand is already being dealt underneath — readability no longer depends on
 * delaying the deal.
 */
test.describe("hand result banner", () => {
  test("shows the winner for ~7s over the next deal, and is dismissible", async ({ browser }) => {
    const a = await freshContext(browser);
    const b = await freshContext(browser);
    await Promise.all([demoSignIn(a.page), demoSignIn(b.page)]);
    await startOpenRoomGame(a, b);

    // End hand 1 by folding whoever acts first.
    const a1 = a.page.getByRole("button", { name: ACTION_RX }).first();
    const b1 = b.page.getByRole("button", { name: ACTION_RX }).first();
    const acted = await Promise.race([
      a1.waitFor({ state: "visible", timeout: 60_000 }).then(() => "A" as const),
      b1.waitFor({ state: "visible", timeout: 60_000 }).then(() => "B" as const),
    ]);
    const actingPage = acted === "A" ? a.page : b.page;
    await actingPage.getByRole("button", { name: /^FOLD$/ }).click();

    // Both players get the banner, headlining the winner.
    for (const page of [a.page, b.page]) {
      await expect(page.getByTestId("hand-banner")).toBeVisible({ timeout: 30_000 });
      await expect(page.getByTestId("hand-banner")).toContainText(/wins \d+/);
      await expect(page.getByTestId("hand-banner-count")).toContainText(/\ds/);
    }

    // The next hand deals underneath while the banner is still up: the phase
    // returns to preflop and hole cards arrive with the banner visible.
    await expect(a.page.getByTestId("phase")).toHaveText(/preflop/, { timeout: 6_000 });
    await expect(a.page.getByTestId("hand-banner")).toBeVisible();

    // One side dismisses early; the other watches it expire on its own.
    await a.page.getByTestId("hand-banner-dismiss").click();
    await expect(a.page.getByTestId("hand-banner")).toHaveCount(0);
    await expect(b.page.getByTestId("hand-banner")).toHaveCount(0, { timeout: 10_000 });

    await a.ctx.close();
    await b.ctx.close();
  });
});
