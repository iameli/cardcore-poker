import { test, expect } from "@playwright/test";
import { demoSignIn, freshContext, startOpenRoomGame } from "./helpers";

const ACTION_RX = /^(FOLD|CHECK|CALL|RAISE|ALL IN)$/;

/**
 * Online-poker pre-action checkboxes: while it's NOT your turn you can arm
 * "CALL <amount>" (fires only at that exact price) or "CALL ANY" (calls
 * whatever's in front of you, checks if nothing is). Armed actions fire the
 * moment action reaches you.
 *
 * 3-player seating is deterministic: host seats 0 (button/UTG preflop),
 * joiners seat in join order — b is SB (owes 10 more), c is BB (owes 0).
 */
test.describe("pre-action checkboxes", () => {
  test("CALL N and CALL ANY act automatically when action reaches you", async ({ browser }) => {
    test.setTimeout(120_000);
    const a = await freshContext(browser);
    const b = await freshContext(browser);
    const c = await freshContext(browser);
    await Promise.all([demoSignIn(a.page), demoSignIn(b.page), demoSignIn(c.page)]);
    await startOpenRoomGame(a, b, c);

    // UTG (the host) is first to act; the blinds are waiting.
    await expect(a.page.getByRole("button", { name: ACTION_RX }).first()).toBeVisible({
      timeout: 60_000,
    });

    // SB is facing 10 more → "CALL 10" is offered; arm it.
    await expect(b.page.getByTestId("preact-call")).toBeVisible({ timeout: 30_000 });
    await expect(b.page.locator(".preact", { hasText: "CALL 10" })).toBeVisible();
    await b.page.getByTestId("preact-call").check();

    // BB owes nothing → no "CALL N", only "CALL ANY"; arm it.
    await expect(c.page.getByTestId("preact-call-any")).toBeVisible({ timeout: 30_000 });
    await expect(c.page.getByTestId("preact-call")).toHaveCount(0);
    await c.page.getByTestId("preact-call-any").check();

    // UTG calls — the armed blinds act entirely on their own and the flop
    // comes with no further clicks: SB auto-calls 10, BB auto-checks.
    await a.page.getByRole("button", { name: /^CALL$/ }).click();
    for (const page of [a.page, b.page, c.page]) {
      await expect(page.getByTestId("phase")).toHaveText(/flop/, { timeout: 30_000 });
    }
    await expect(b.page.locator(".log-entry", { hasText: "You: call" }).first()).toBeVisible();
    await expect(c.page.locator(".log-entry", { hasText: "You: check" }).first()).toBeVisible();

    // Pre-actions are one-shot: both checkboxes cleared after firing.
    await expect(b.page.getByTestId("preact-call-any")).not.toBeChecked();
    await expect(c.page.getByTestId("preact-call-any")).not.toBeChecked();

    await a.ctx.close();
    await b.ctx.close();
    await c.ctx.close();
  });
});
