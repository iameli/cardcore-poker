import { expect, Browser, Page } from "@playwright/test";

export type Ctx = { ctx: Awaited<ReturnType<Browser["newContext"]>>; page: Page };

export async function freshContext(browser: Browser): Promise<Ctx> {
  const ctx = await browser.newContext();
  const page = await ctx.newPage();
  return { ctx, page };
}

export async function demoSignIn(page: Page) {
  await page.goto("/");
  await page.getByRole("button", { name: /Play in Demo Mode/i }).click();
  await expect(page.getByRole("heading", { name: /^Lobby$/i })).toBeVisible({
    timeout: 15_000,
  });
}

export async function readHandle(page: Page): Promise<string> {
  return (await page.locator(".name").first().innerText()).trim();
}

/**
 * Drive the open-room flow end to end: `a` hosts a room, every other context
 * opens its URL and requests to join, `a` approves each and starts. Resolves
 * once all players are in the GameRoom. Returns the table's AT URI.
 */
export async function startOpenRoomGame(a: Ctx, ...joiners: Ctx[]): Promise<string> {
  await a.page.getByTestId("create-open-room").click();
  await expect(a.page.getByTestId("copy-table-uri")).toBeVisible({ timeout: 15_000 });
  const tid = (await a.page.getByTestId("copy-table-uri").locator("code").innerText())
    .trim()
    .split("/")
    .pop()!;
  const didA = await a.page.evaluate(
    () => JSON.parse(localStorage.getItem("cardcore_demo_session")!).did,
  );
  const tableUri = `at://${didA}/re.cardco.poker.table/${tid}`;

  // Join + approve sequentially so the roster order (and therefore seating)
  // is deterministic: host, then joiners in the order given.
  for (let i = 0; i < joiners.length; i++) {
    const j = joiners[i];
    await j.page.goto(`/${tableUri}`);
    await j.page.getByTestId("request-join").click();
    await expect(a.page.getByTestId("approve-request").first()).toBeVisible({ timeout: 30_000 });
    await a.page.getByTestId("approve-request").first().click();
    await expect(a.page.getByTestId("approve-request")).toHaveCount(0);
  }
  await expect(a.page.getByTestId("start-game")).toBeEnabled();
  await a.page.getByTestId("start-game").click();
  for (const ctx of [a, ...joiners]) {
    await expect(ctx.page.getByTestId("phase")).toBeVisible({ timeout: 30_000 });
  }
  return tableUri;
}
