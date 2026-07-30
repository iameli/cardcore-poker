import { test, expect } from "@playwright/test";
import { demoSignIn, freshContext, startOpenRoomGame } from "./helpers";

const ACTION_RX = /^(FOLD|CHECK|CALL|RAISE|ALL IN)$/;
const PDS = "http://localhost:2583";

/**
 * A player opens their own game on a DIFFERENT device: same identity (the
 * demo session is copied over), but the per-table seed in localStorage —
 * the game's key material — is not there. The client must recognize it
 * can't reproduce its published history and fall back to spectating the
 * replay, rather than fabricating a fresh seed and trying to publish
 * divergent actions into already-occupied slots.
 */
test.describe("keyless device", () => {
  test("opening your own game without its key material spectates instead of diverging", async ({
    browser,
  }) => {
    test.setTimeout(120_000);
    const a = await freshContext(browser);
    const b = await freshContext(browser);
    await Promise.all([demoSignIn(a.page), demoSignIn(b.page)]);
    const tableUri = await startOpenRoomGame(a, b);

    // Play hand 1 to completion (first to act folds).
    const a1 = a.page.getByRole("button", { name: ACTION_RX }).first();
    const b1 = b.page.getByRole("button", { name: ACTION_RX }).first();
    const acted = await Promise.race([
      a1.waitFor({ state: "visible", timeout: 60_000 }).then(() => "A" as const),
      b1.waitFor({ state: "visible", timeout: 60_000 }).then(() => "B" as const),
    ]);
    const actingPage = acted === "A" ? a.page : b.page;
    await actingPage.getByRole("button", { name: /^FOLD$/ }).click();
    await expect(a.page.getByTestId("hand-banner")).toBeVisible({ timeout: 30_000 });

    // "Another computer": a fresh browser context with B's identity but
    // WITHOUT B's localStorage (no per-table seed).
    const session = await b.page.evaluate(() => localStorage.getItem("cardcore_demo_session"));
    const recordCount = async () => {
      const did = JSON.parse(session!).did;
      const res = await fetch(
        `${PDS}/xrpc/com.atproto.repo.listRecords?repo=${did}` +
          `&collection=re.cardco.poker.action&limit=100`,
      );
      return (await res.json()).records.length;
    };
    const before = await recordCount();

    const laptop = await freshContext(browser);
    await laptop.page.addInitScript((s: string) => {
      localStorage.setItem("cardcore_demo_session", s);
    }, session!);
    await laptop.page.goto(`/${tableUri}`);

    // The client detects the missing key material and watches instead.
    await expect(laptop.page.getByTestId("keyless-note")).toBeVisible({ timeout: 30_000 });
    await expect(laptop.page.getByTestId("keyless-note")).toContainText(/keys aren't on this/);

    // The replay itself works — hand 1's result appears in the log — and no
    // divergence error is thrown.
    await expect(
      laptop.page.locator(".log-entry", { hasText: /Hand 1 results/ }).first(),
    ).toBeVisible({ timeout: 30_000 });
    await expect(laptop.page.locator(".error-banner")).toHaveCount(0);

    // No betting UI is ever offered on the keyless device…
    await expect(laptop.page.getByRole("button", { name: ACTION_RX })).toHaveCount(0);

    // …and it wrote NOTHING to the repo: replay is read-only.
    expect(await recordCount()).toBe(before);

    // Harder case: a seed IS present but it's the WRONG one (stale backup,
    // synced localStorage from another table…). Presence isn't enough — the
    // seed must reproduce our published seq-0 commit, or we spectate.
    const wrongSeed = Array.from({ length: 32 }, (_, i) => i).join(",");
    const laptop2 = await freshContext(browser);
    await laptop2.page.addInitScript(
      ({ s, uri, seed }: { s: string; uri: string; seed: string }) => {
        localStorage.setItem("cardcore_demo_session", s);
        localStorage.setItem(`cardcore_seed:${uri}`, seed);
      },
      { s: session!, uri: tableUri, seed: wrongSeed },
    );
    await laptop2.page.goto(`/${tableUri}`);
    await expect(laptop2.page.getByTestId("keyless-note")).toBeVisible({ timeout: 30_000 });
    expect(await recordCount()).toBe(before);

    await a.ctx.close();
    await b.ctx.close();
    await laptop.ctx.close();
    await laptop2.ctx.close();
  });
});
