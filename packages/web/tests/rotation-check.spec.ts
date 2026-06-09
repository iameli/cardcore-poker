import { test, expect, Browser, Page } from "@playwright/test";

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

async function demoSignIn(page: Page) {
  await page.goto("/");
  await page.getByRole("button", { name: /Play in Demo Mode/i }).click();
  await expect(page.getByRole("heading", { name: /^Lobby$/i })).toBeVisible({
    timeout: 15_000,
  });
}

async function readHandle(page: Page): Promise<string> {
  return (await page.locator(".name").first().innerText()).trim();
}

test("local player is in the bottom-row from each perspective", async ({ browser }) => {
  const a = await freshContext(browser);
  const b = await freshContext(browser);
  await Promise.all([demoSignIn(a.page), demoSignIn(b.page)]);
  const handleB = await readHandle(b.page);

  await a.page.getByTestId("opponent-handle").fill(handleB);
  await a.page.getByTestId("create-table").click();
  await expect(a.page.getByTestId("copy-table-uri")).toBeVisible({ timeout: 15_000 });

  const tid = (await a.page.getByTestId("copy-table-uri").locator("code").innerText())
    .trim()
    .split("/")
    .pop()!;
  const didA = await a.page.evaluate(
    () => JSON.parse(localStorage.getItem("cardcore_demo_session")!).did,
  );
  const uri = `at://${didA}/re.cardco.poker.table/${tid}`;

  await b.page.getByTestId("join-uri").fill(uri);
  await b.page.getByTestId("join-table").click();
  await expect(b.page.getByTestId("copy-table-uri")).toBeVisible({ timeout: 15_000 });

  for (const [tag, page] of [
    ["A", a.page],
    ["B", b.page],
  ] as const) {
    const selfInBottom = await page.locator(".seat-area.bottom .seat.self").count();
    const selfElsewhere = await page.locator(".seat-area:not(.bottom) .seat.self").count();
    // With 2 players the opponent should be at top-middle (opposite the user).
    const opponentInTop = await page.locator(".seat-area.top .seat:not(.self):not(.empty)").count();
    console.log(
      `${tag}: self-in-bottom=${selfInBottom} self-elsewhere=${selfElsewhere} opponent-in-top=${opponentInTop}`,
    );
    expect(selfInBottom, `${tag} should have a .seat.self inside .seat-area.bottom`).toBe(1);
    expect(selfElsewhere, `${tag} should NOT have a .seat.self in any other row`).toBe(0);
    expect(opponentInTop, `${tag} should have the opponent rendered in the top row`).toBe(1);
  }

  await a.ctx.close();
  await b.ctx.close();
});
