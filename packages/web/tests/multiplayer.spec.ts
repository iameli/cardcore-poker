import { test, expect, Page, Browser } from "@playwright/test";

/**
 * E2E multiplayer test: two browser contexts each create a real account on
 * the local PDS, one creates a table referencing the other's handle, the
 * other joins via the table's AT URI. Validates that the cryptographic
 * dealing protocol completes via PDS-only transport (no relay server).
 */

async function demoSignIn(page: Page) {
  await page.goto("/");
  await page.getByRole("button", { name: /Play in Demo Mode/i }).click();
  await expect(page.getByRole("heading", { name: /^Lobby$/i })).toBeVisible({
    timeout: 15_000,
  });
}

async function readHandle(page: Page): Promise<string> {
  const text = await page.locator(".name").first().innerText();
  return text.trim();
}

async function freshContext(browser: Browser) {
  const ctx = await browser.newContext();
  const page = await ctx.newPage();
  return { ctx, page };
}

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

    // A creates a table that includes B
    await a.page.getByTestId("opponent-handle").fill(handleB);
    await a.page.getByTestId("create-table").click();

    // After create, A is in GameRoom; pull the table URI from the share button
    await expect(a.page.getByTestId("copy-table-uri")).toBeVisible({ timeout: 15_000 });
    const tableTid = await a.page.getByTestId("copy-table-uri").locator("code").innerText();
    // Reconstruct the URI from A's DID and the tid
    const didA = await a.page.evaluate(
      () => JSON.parse(localStorage.getItem("cardcore_demo_session")!).did,
    );
    const tableUri = `at://${didA}/re.cardco.poker.table/${tableTid}`;
    console.log(`tableUri=${tableUri}`);

    // B joins via the URI
    await b.page.getByTestId("join-uri").fill(tableUri);
    await b.page.getByTestId("join-table").click();
    await expect(b.page.getByTestId("copy-table-uri")).toBeVisible({ timeout: 15_000 });

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

    // Fold and verify the hand reaches Showdown/Complete on both sides
    await actingPage.getByRole("button", { name: /^FOLD$/ }).click();
    for (const page of [a.page, b.page]) {
      await expect(page.getByTestId("phase")).toHaveText(/showdown/, { timeout: 30_000 });
    }

    await a.ctx.close();
    await b.ctx.close();
  });
});
