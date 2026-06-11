import { test, expect, Page } from "@playwright/test";
import { demoSignIn, freshContext, readHandle, startOpenRoomGame } from "./helpers";

/**
 * E2E multiplayer test: two browser contexts each create a real account on
 * the local PDS and play a hand together via the open-room flow. Validates
 * that the cryptographic dealing protocol completes via PDS-only transport
 * (no relay server), plus assorted in-game UI behaviors.
 */

const ACTION_RX = /^(FOLD|CHECK|CALL|RAISE|ALL IN)$/;

test.describe("multiplayer (PDS-only)", () => {
  test("two demo players play one hand end-to-end", async ({ browser }) => {
    const a = await freshContext(browser);
    const b = await freshContext(browser);

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

    // Sign in two distinct demo accounts
    await Promise.all([demoSignIn(a.page), demoSignIn(b.page)]);

    const handleA = await readHandle(a.page);
    const handleB = await readHandle(b.page);
    expect(handleA).not.toBe(handleB);
    console.log(`A=${handleA}  B=${handleB}`);

    const tableUri = await startOpenRoomGame(a, b);
    console.log(`tableUri=${tableUri}`);

    // The table URL persists for the whole game on both sides.
    await expect(a.page).toHaveURL(/\/at:\/\//);
    await expect(b.page).toHaveURL(/\/at:\/\//);

    // The header copy button shows host/tid and puts the FULL shareable URL
    // on the clipboard — ready to paste into a browser.
    await a.ctx.grantPermissions(["clipboard-read", "clipboard-write"]);
    await expect(a.page.getByTestId("copy-table-uri").locator("code")).toContainText("/");
    await a.page.getByTestId("copy-table-uri").click();
    const copiedUrl = await a.page.evaluate(() => navigator.clipboard.readText());
    expect(copiedUrl).toBe(`${new URL(a.page.url()).origin}/${tableUri}`);

    // The cryptographic deal runs over the PDS. One player's UI eventually
    // surfaces an action button when it's their turn.
    const a1 = a.page.getByRole("button", { name: ACTION_RX }).first();
    const b1 = b.page.getByRole("button", { name: ACTION_RX }).first();
    const acted = await Promise.race([
      a1.waitFor({ state: "visible", timeout: 60_000 }).then(() => "A" as const),
      b1.waitFor({ state: "visible", timeout: 60_000 }).then(() => "B" as const),
    ]);
    console.log(`Acting first: ${acted}`);

    const actingPage = acted === "A" ? a.page : b.page;
    await expect(actingPage.getByRole("button", { name: /^RAISE$/ })).toBeVisible();

    // The waiting player's bar names the exact protocol step and who owes it
    // — the always-on stall-debugging line.
    const waitingPage = acted === "A" ? b.page : a.page;
    const actingHandle = acted === "A" ? handleA : handleB;
    await expect(waitingPage.getByTestId("waiting-on")).toContainText(
      `waiting on bet from ${actingHandle}`,
      { timeout: 15_000 },
    );

    // The layout holds still across turn changes: the acting player's bar
    // (buttons) and the waiting player's bar (placeholder text) are the same
    // height, so the scaled game doesn't jump.
    const heightOf = (p: Page) =>
      p.locator(".fit-content").evaluate((el) => (el as HTMLElement).clientHeight);
    expect(Math.abs((await heightOf(a.page)) - (await heightOf(b.page)))).toBeLessThanOrEqual(1);

    // DIDs should only be a fallback — A's table shows B by handle.
    await expect(a.page.getByText(handleB).first()).toBeVisible({ timeout: 15_000 });

    // Noninteractive protocol steps are visible in the log too.
    await expect(
      a.page
        .locator(".log-entry")
        .filter({ hasText: /commitSeed/ })
        .first(),
    ).toBeVisible();

    // Fold and verify the hand reaches Showdown/Complete on both sides
    await actingPage.getByRole("button", { name: /^FOLD$/ }).click();
    for (const page of [a.page, b.page]) {
      await expect(page.getByTestId("phase")).toHaveText(/showdown/, { timeout: 30_000 });
    }

    // The log anchors to the bottom as entries stream in… (shrink the window
    // so the log is guaranteed to overflow its panel)
    await a.page.setViewportSize({ width: 1100, height: 360 });
    const log = a.page.locator(".log-entries");
    await expect
      .poll(() => log.evaluate((el) => el.scrollHeight - el.scrollTop - el.clientHeight < 10))
      .toBe(true);
    await expect(a.page.getByTestId("log-jump-live")).toHaveCount(0);
    // …until you scroll up, which shows the return-to-live arrow…
    await log.evaluate((el) => {
      el.scrollTop = 0;
    });
    await expect(a.page.getByTestId("log-jump-live")).toBeVisible();
    // …and clicking it re-anchors.
    await a.page.getByTestId("log-jump-live").click();
    await expect(a.page.getByTestId("log-jump-live")).toHaveCount(0);
    await expect
      .poll(() => log.evaluate((el) => el.scrollHeight - el.scrollTop - el.clientHeight < 10))
      .toBe(true);

    await a.ctx.close();
    await b.ctx.close();
  });
});
