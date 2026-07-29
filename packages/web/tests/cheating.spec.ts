import { test, expect, Page } from "@playwright/test";
import { demoSignIn, freshContext, startOpenRoomGame } from "./helpers";

const ACTION_RX = /^(FOLD|CHECK|CALL|RAISE|ALL IN)$/;
const PDS = "http://localhost:2583";

/**
 * A player CHEATS: their client is honest, but "they" (this test, using their
 * own credentials) push a forged action record straight to their repo at the
 * next expected rkey — an under-raise below the standing bet, the exact class
 * of action the engine must never accept. Every client at the table must
 * flag the violation in the UI rather than silently buffering or applying it.
 */
test.describe("cheat detection", () => {
  test("a forged under-raise record raises the violation banner on every client", async ({
    browser,
  }) => {
    test.setTimeout(120_000);
    const a = await freshContext(browser);
    const b = await freshContext(browser);
    await Promise.all([demoSignIn(a.page), demoSignIn(b.page)]);
    const tableUri = await startOpenRoomGame(a, b);
    const tid = tableUri.split("/").pop()!;

    // Identify the acting player (heads-up: the SB acts first preflop).
    const a1 = a.page.getByRole("button", { name: ACTION_RX }).first();
    const b1 = b.page.getByRole("button", { name: ACTION_RX }).first();
    const acted = await Promise.race([
      a1.waitFor({ state: "visible", timeout: 60_000 }).then(() => "A" as const),
      b1.waitFor({ state: "visible", timeout: 60_000 }).then(() => "B" as const),
    ]);
    const actingPage = acted === "A" ? a.page : b.page;
    const cheaterPage = acted === "A" ? b.page : a.page;

    // SB calls; action moves to the BB — the cheater. Their client dutifully
    // shows bet buttons; instead, a forged record lands in their repo.
    await actingPage.getByRole("button", { name: /^CALL$/ }).click();
    await expect(cheaterPage.getByRole("button", { name: ACTION_RX }).first()).toBeVisible({
      timeout: 30_000,
    });

    const cheater = await cheaterPage.evaluate(() =>
      JSON.parse(localStorage.getItem("cardcore_demo_session")!),
    );

    // Next expected seq = number of actions already in the cheater's repo for
    // this table (seqs are contiguous from 0).
    const list = await (
      await fetch(
        `${PDS}/xrpc/com.atproto.repo.listRecords?repo=${cheater.did}` +
          `&collection=re.cardco.poker.action&limit=100`,
      )
    ).json();
    const seq = list.records.filter((r: any) =>
      r.uri.split("/").pop().startsWith(`${tid}-`),
    ).length;

    const table = await (
      await fetch(
        `${PDS}/xrpc/com.atproto.repo.getRecord?repo=${tableUri.split("/")[2]}` +
          `&collection=re.cardco.poker.table&rkey=${tid}`,
      )
    ).json();

    // The forgery: "raise" to a total of 5 while the standing bet is 20 —
    // the under-raise that once wedged a real game. Published with the
    // cheater's own credentials at their next expected rkey.
    const res = await fetch(`${PDS}/xrpc/com.atproto.repo.createRecord`, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        authorization: `Bearer ${cheater.accessJwt}`,
      },
      body: JSON.stringify({
        repo: cheater.did,
        collection: "re.cardco.poker.action",
        rkey: `${tid}-${String(seq).padStart(9, "0")}`,
        record: {
          $type: "re.cardco.poker.action",
          table: { uri: tableUri, cid: table.cid },
          seq,
          action: { $type: "re.cardco.poker.defs#bet", action: "raise", amount: 5 },
          createdAt: new Date().toISOString(),
        },
      }),
    });
    expect(res.ok).toBe(true);

    // Every client at the table flags it — including the cheater's own.
    for (const page of [a.page, b.page]) {
      await expect(page.getByTestId("violation-banner")).toBeVisible({ timeout: 30_000 });
      await expect(page.getByTestId("violation-entry").first()).toContainText(/below the minimum/);
    }
    // The honest player sees the cheater named; the cheater sees "You".
    const honestPage = acted === "A" ? a.page : b.page;
    await expect(honestPage.getByTestId("violation-entry").first()).toContainText(cheater.handle);
    await expect(cheaterPage.getByTestId("violation-entry").first()).toContainText(
      /^You published/,
    );

    // The violation also lands in the game log for the record.
    await expect(
      honestPage.locator(".log-entry", { hasText: "INVALID ACTION" }).first(),
    ).toBeVisible();

    await a.ctx.close();
    await b.ctx.close();
  });
});
