import { test, expect } from "@playwright/test";
import { demoSignIn, freshContext, startOpenRoomGame } from "./helpers";

/**
 * E2E reload-resume test: a player reloads mid-hand and rejoins the live
 * game. The resumed session re-derives its crypto from the persisted seed,
 * replays its own past records from its repo (including bets, which aren't
 * re-derivable), replays the peers' records, and keeps playing.
 */

const ACTION_RX = /^(FOLD|CHECK|CALL|RAISE|ALL IN)$/;

test.describe("reload-resume (PDS-only)", () => {
  test("a player reloads mid-hand and the game continues", async ({ browser }) => {
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
    await startOpenRoomGame(a, b);

    // Whoever acts first CALLS — putting a non-re-derivable bet on their repo
    // — then reloads while the other player is on the clock.
    const a1 = a.page.getByRole("button", { name: ACTION_RX }).first();
    const b1 = b.page.getByRole("button", { name: ACTION_RX }).first();
    const acted = await Promise.race([
      a1.waitFor({ state: "visible", timeout: 60_000 }).then(() => "A" as const),
      b1.waitFor({ state: "visible", timeout: 60_000 }).then(() => "B" as const),
    ]);
    const x = acted === "A" ? a : b; // reloader
    const y = acted === "A" ? b : a; // stays live

    await x.page.getByRole("button", { name: /^CALL$/ }).click();
    await y.page
      .getByRole("button", { name: ACTION_RX })
      .first()
      .waitFor({ state: "visible", timeout: 30_000 });

    // Reload X mid-hand. The table URL routes straight back into the game,
    // and the session resumes as a PLAYER (not a spectator).
    await x.page.reload();
    await expect(x.page.getByTestId("phase")).toBeVisible({ timeout: 30_000 });
    await expect(x.page.getByTestId("spectating")).toHaveCount(0);

    // Y folds; X's resumed session sees the hand through to showdown.
    await y.page.getByRole("button", { name: /^FOLD$/ }).click();
    for (const page of [x.page, y.page]) {
      await expect(page.getByTestId("phase")).toHaveText(/showdown/, { timeout: 30_000 });
    }

    await a.ctx.close();
    await b.ctx.close();
  });
});
