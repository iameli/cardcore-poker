import { test, expect, Page, Browser } from "@playwright/test";

/**
 * E2E join-request bump test: discovery is a live stream with no backfill, so
 * a host that misses the original join-request commit (dropped socket, page
 * reload, late subscribe) would never see the joiner. Joiners therefore
 * re-publish their request with a bumped updatedAt every 15s while waiting.
 *
 * Here the host RELOADS after the joiner's request was published — the
 * restarted watcher missed it — and discovery still happens on the next bump.
 */

async function demoSignIn(page: Page) {
  await page.goto("/");
  await page.getByRole("button", { name: /Play in Demo Mode/i }).click();
  await expect(page.getByRole("heading", { name: /^Lobby$/i })).toBeVisible({
    timeout: 15_000,
  });
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

test.describe("join-request bump (PDS-only)", () => {
  test("a host that missed the join request discovers it via the bump", async ({ browser }) => {
    test.setTimeout(90_000);

    const a = await freshContext(browser); // host
    const b = await freshContext(browser); // joiner

    // Spotty-connection simulation: while `blackout` is on, every firehose
    // frame headed for A is silently dropped — like a train tunnel. Frames
    // are NOT redelivered after the blackout (it's a live stream), so A can
    // only learn about B from a commit made after the blackout ends.
    let blackout = false;
    await a.page.routeWebSocket(/subscribeRepos/, (ws) => {
      const server = ws.connectToServer();
      ws.onMessage((msg) => server.send(msg));
      server.onMessage((msg) => {
        if (!blackout) ws.send(msg);
      });
    });

    await Promise.all([demoSignIn(a.page), demoSignIn(b.page)]);

    // A opens a room.
    await a.page.getByTestId("create-open-room").click();
    await expect(a.page.getByTestId("copy-table-uri")).toBeVisible({ timeout: 15_000 });
    const tid = (await a.page.getByTestId("copy-table-uri").locator("code").innerText())
      .trim()
      .split("/")
      .pop()!;
    const didA = await a.page.evaluate(
      () => JSON.parse(localStorage.getItem("cardcore_demo_session")!).did,
    );
    const tableUri = `at://${didA}/re.cardco.poker.table/${tid.trim()}`;

    // B requests to join during A's blackout — A misses the commit entirely.
    blackout = true;
    await b.page.goto(`/${tableUri}`);
    await b.page.getByTestId("request-join").click();
    await expect(b.page.getByTestId("join-status")).toBeVisible({ timeout: 15_000 });
    await a.page.waitForTimeout(2_000);
    await expect(a.page.getByTestId("no-requests")).toBeVisible();
    blackout = false;

    // Connectivity is back, but the original event is gone for good. B's 15s
    // updatedAt bump re-publishes the request, and A discovers it.
    await expect(a.page.getByTestId("approve-request")).toBeVisible({ timeout: 25_000 });

    // The bump-discovered request flows through approval and start as normal.
    await a.page.getByTestId("approve-request").click();
    await a.page.getByTestId("start-game").click();
    await expect(a.page.getByTestId("phase")).toBeVisible({ timeout: 30_000 });
    await expect(b.page.getByTestId("phase")).toBeVisible({ timeout: 30_000 });

    await a.ctx.close();
    await b.ctx.close();
  });
});
