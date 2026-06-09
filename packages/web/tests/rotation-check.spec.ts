import { test, expect } from "@playwright/test";
import { demoSignIn, freshContext, startOpenRoomGame } from "./helpers";

test("local player is in the bottom-row from each perspective", async ({ browser }) => {
  const a = await freshContext(browser);
  const b = await freshContext(browser);
  await Promise.all([demoSignIn(a.page), demoSignIn(b.page)]);
  await startOpenRoomGame(a, b);

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
