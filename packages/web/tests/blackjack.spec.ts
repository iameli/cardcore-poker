import { expect, test } from "@playwright/test";
import { Ctx, demoSignIn, freshContext, startOpenRoomGame } from "./helpers";

/**
 * Blackjack end-to-end: two demo players create a blackjack room through the
 * open-room consensus flow, play a full provably-fair round (prefer-STAND
 * policy, insurance declined), read the results, and the game auto-advances
 * to round 2 with the banker rotated.
 *
 * Outcomes are card-dependent, so assertions are structural: legal moves
 * only, result log present, banker cards rendered, rotation observed.
 */
test.describe("blackjack (PDS-only)", () => {
  test.setTimeout(300_000);

  test("two demo players play blackjack rounds with a rotating banker", async ({ browser }) => {
    const a = await freshContext(browser);
    const b = await freshContext(browser);
    try {
      await demoSignIn(a.page);
      await demoSignIn(b.page);

      // Lobby picker → blackjack room → consensus flow → game.
      const tableUri = await startOpenRoomGame(a, b, { game: "blackjack" });
      expect(tableUri).toContain("re.cardco.blackjack.table");

      // ── Round 1: the host (seat 0) banks; b is the bettor ──
      await expect(b.page.getByTestId("wager-panel")).toBeVisible({ timeout: 90_000 });
      // The banker never wagers — no panel on a's side.
      await expect(a.page.getByTestId("wager-panel")).toHaveCount(0);
      // The banker seat is flagged on the table.
      await expect(b.page.getByTestId("banker-seat")).toBeVisible();

      await b.page.getByTestId("wager-submit").click();

      // Play out the bettor's turn; the banker then draws automatically.
      await playUntilRoundResult(b);
      await expect(a.page.getByTestId("round-result")).toBeVisible({ timeout: 90_000 });

      // Both players render the banker's completed hand (>= 2 cards) + total.
      for (const p of [a, b]) {
        const cards = p.page.getByTestId("banker-cards").locator(".card");
        expect(await cards.count()).toBeGreaterThanOrEqual(2);
        await expect(p.page.getByTestId("banker-total")).toBeVisible();
      }

      // The round-result log line shows a real outcome for the bettor.
      await expect(
        b.page.getByText(/ — (win|lose|push|blackjack|bust|surrender)/).first(),
      ).toBeVisible();
      await expect(a.page.getByText(/— Round 1 results —/)).toBeVisible();

      // ── Auto-advance: round 2 starts with the banker rotated to b, so a
      // is now the bettor and gets the wager panel. ──
      await expect(a.page.getByTestId("wager-panel")).toBeVisible({ timeout: 90_000 });
      await expect(a.page.getByTestId("phase")).toHaveText("wagering");
      await expect(b.page.getByTestId("wager-panel")).toHaveCount(0);

      // ── Round 2 plays to completion the same way ──
      await a.page.getByTestId("wager-submit").click();
      await playUntilRoundResult(a);
      await expect(b.page.getByText(/— Round 2 results —/)).toBeVisible({ timeout: 90_000 });
    } finally {
      await a.ctx.close();
      await b.ctx.close();
    }
  });
});

/**
 * Drive the bettor until the round result shows: decline insurance when
 * offered, otherwise STAND (always legal alongside HIT — asserted). A
 * natural blackjack auto-stands, so the loop may see no decision at all.
 */
async function playUntilRoundResult(p: Ctx) {
  for (let i = 0; i < 240; i++) {
    if (
      await p.page
        .getByTestId("round-result")
        .isVisible()
        .catch(() => false)
    )
      return;

    // Clicks are bounded: a submitted decision keeps its panel up until the
    // publish echo clears it, so a second click can start against a button
    // that detaches mid-actionability-check — unbounded, it would hang the
    // poll loop right through the round-result window.
    const noInsurance = p.page.getByRole("button", { name: "NO INSURANCE" });
    if (await noInsurance.isVisible().catch(() => false)) {
      await noInsurance.click({ timeout: 1500 }).catch(() => {});
      continue;
    }

    const stand = p.page.getByRole("button", { name: /^STAND$/ });
    if (await stand.isVisible().catch(() => false)) {
      // Legal-move sanity: HIT accompanies STAND; poker's FOLD never shows.
      // Sampled without waiting — the panel may detach mid-iteration once a
      // previously clicked decision lands, and that's not a rules violation.
      // A genuinely HIT-less panel never gets clicked, so the loop's
      // iteration cap still fails the round.
      const hitVisible = await p.page
        .getByRole("button", { name: /^HIT$/ })
        .isVisible()
        .catch(() => false);
      if (hitVisible) {
        expect(await p.page.getByRole("button", { name: /^FOLD$/ }).count()).toBe(0);
        await stand.click({ timeout: 1500 }).catch(() => {});
      }
      continue;
    }

    await p.page.waitForTimeout(500);
  }
  throw new Error("blackjack round did not reach a result");
}
