import { test, expect } from "@playwright/test";
import { demoSignIn, freshContext, startOpenRoomGame } from "./helpers";

/**
 * Noninteractive protocol actions (commitSeed, shuffleDeck, lockDeck,
 * revealLockKey) fold into collapsible groups in the game log: the hand's
 * human story stays readable, and every protocol step stays inspectable.
 */
test.describe("game log protocol folds", () => {
  test("protocol steps are folded by default and expandable for inspection", async ({
    browser,
  }) => {
    const a = await freshContext(browser);
    const b = await freshContext(browser);
    await Promise.all([demoSignIn(a.page), demoSignIn(b.page)]);
    await startOpenRoomGame(a, b);

    // The deal's protocol steps arrive folded — no raw protocol entries.
    const firstFold = a.page.getByTestId("log-fold").first();
    await expect(firstFold).toBeVisible({ timeout: 30_000 });
    await expect(firstFold).toContainText(/\d+ protocol step/);
    await expect(a.page.getByTestId("log-protocol-entry")).toHaveCount(0);

    // Expanding shows the individual steps (commitSeed, reveals, …).
    await firstFold.click();
    const entries = a.page.getByTestId("log-protocol-entry");
    await expect(entries.first()).toBeVisible();
    await expect(
      entries.filter({ hasText: /commitSeed|shuffleDeck|revealLockKey/ }).first(),
    ).toBeVisible();

    // Collapsing hides them again.
    await firstFold.click();
    await expect(a.page.getByTestId("log-protocol-entry")).toHaveCount(0);

    await a.ctx.close();
    await b.ctx.close();
  });
});
