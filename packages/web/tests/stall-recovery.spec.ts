import { test, expect } from "@playwright/test";
import { demoSignIn, freshContext, startOpenRoomGame } from "./helpers";

/**
 * E2E stall-recovery test: the firehose is a live stream — a frame dropped by
 * a flaky connection is gone for good. Without recovery, a player who misses
 * a peer's record (worst case: the burst of records right at game start)
 * waits forever while everyone else waits on them — the classic "everyone
 * committed but nobody shuffled" stall.
 *
 * Here B's firehose silently drops EVERY incoming frame while the deal
 * starts. Two things must happen:
 *   1. B's always-on status line names the exact step and player it's
 *      blocked on (the stall is visible, not a frozen table).
 *   2. The quiet-period backfill sweep recovers the lost records straight
 *      from the PDS, so the hand completes anyway.
 */

const ACTION_RX = /^(FOLD|CHECK|CALL|RAISE|ALL IN)$/;

test.describe("stall recovery (PDS-only)", () => {
  test("a player who misses firehose frames is unstuck by the quiet-period sweep", async ({
    browser,
  }) => {
    test.setTimeout(120_000);

    const a = await freshContext(browser); // host — healthy connection
    const b = await freshContext(browser); // joiner — drops all incoming frames

    // Spotty-connection simulation: while `blackout` is on, every firehose
    // frame headed for B is silently dropped — the socket stays open, so no
    // reconnect/cursor-resume kicks in. Dropped frames are NOT redelivered.
    let blackout = true;
    await b.page.routeWebSocket(/subscribeRepos/, (ws) => {
      const server = ws.connectToServer();
      ws.onMessage((msg) => server.send(msg));
      server.onMessage((msg) => {
        if (!blackout) ws.send(msg);
      });
    });

    await Promise.all([demoSignIn(a.page), demoSignIn(b.page)]);
    const tableUri = await startOpenRoomGame(a, b);
    console.log(`tableUri=${tableUri}`);

    // B can publish (HTTP) but hears nothing back, so the deal stalls at the
    // first record B misses — and the protocol alternates, so A stalls too,
    // waiting on responses B can't make. Hold the blackout well past the
    // normal deal time (<2s locally) to guarantee records were actually
    // dropped, then verify a REAL stall: B's status line still names a deal
    // step, and neither player has betting buttons.
    const STALL_RX = /waiting on (commitSeed|shuffleDeck|lockDeck|revealLockKey #\d+) from/;
    await expect(b.page.getByTestId("waiting-on")).toContainText(STALL_RX, { timeout: 20_000 });
    await b.page.waitForTimeout(4_000);
    await expect(b.page.getByTestId("waiting-on")).toContainText(STALL_RX);
    await expect(a.page.getByRole("button", { name: ACTION_RX })).toHaveCount(0);
    await expect(b.page.getByRole("button", { name: ACTION_RX })).toHaveCount(0);

    // Connectivity returns, but the dropped frames are gone for good — only
    // the quiet-period PDS sweep can recover them and unstick the game.
    blackout = false;

    const a1 = a.page.getByRole("button", { name: ACTION_RX }).first();
    const b1 = b.page.getByRole("button", { name: ACTION_RX }).first();
    const acted = await Promise.race([
      a1.waitFor({ state: "visible", timeout: 60_000 }).then(() => "A" as const),
      b1.waitFor({ state: "visible", timeout: 60_000 }).then(() => "B" as const),
    ]);
    console.log(`Acting first after recovery: ${acted}`);

    // The recovered hand plays out to showdown like any other.
    const actingPage = acted === "A" ? a.page : b.page;
    await actingPage.getByRole("button", { name: /^FOLD$/ }).click();
    for (const page of [a.page, b.page]) {
      await expect(page.getByTestId("phase")).toHaveText(/showdown/, { timeout: 30_000 });
    }

    await a.ctx.close();
    await b.ctx.close();
  });
});
