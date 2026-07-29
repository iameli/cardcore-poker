import { test, expect } from "@playwright/test";
import { demoSignIn, freshContext, startOpenRoomGame } from "./helpers";

const ACTION_RX = /^(FOLD|CHECK|CALL|RAISE|ALL IN)$/;

/**
 * A cue fires when it becomes your turn (observable as
 * window.__cardcoreTurnCues, incremented whether or not audio is audible),
 * and a device-local settings menu can turn it off — persisted per device.
 */
test.describe("turn audio cue + settings", () => {
  test("cues on your turn, toggleable in settings, persists across reload", async ({ browser }) => {
    const a = await freshContext(browser);
    const b = await freshContext(browser);
    await Promise.all([demoSignIn(a.page), demoSignIn(b.page)]);
    const tableUri = await startOpenRoomGame(a, b);

    const a1 = a.page.getByRole("button", { name: ACTION_RX }).first();
    const b1 = b.page.getByRole("button", { name: ACTION_RX }).first();
    const acted = await Promise.race([
      a1.waitFor({ state: "visible", timeout: 60_000 }).then(() => "A" as const),
      b1.waitFor({ state: "visible", timeout: 60_000 }).then(() => "B" as const),
    ]);
    const actingPage = acted === "A" ? a.page : b.page;
    const waitingPage = acted === "A" ? b.page : a.page;

    // Sound defaults ON: the acting player got a cue when their turn arrived.
    await expect
      .poll(() => actingPage.evaluate(() => (window as any).__cardcoreTurnCues || 0))
      .toBeGreaterThan(0);
    // The waiting player hasn't had a turn yet — no cue.
    expect(await waitingPage.evaluate(() => (window as any).__cardcoreTurnCues || 0)).toBe(0);

    // The waiting player turns the cue off before their turn arrives.
    await waitingPage.getByTestId("settings-toggle").click();
    await expect(waitingPage.getByTestId("setting-turn-sound")).toBeChecked();
    await waitingPage.getByTestId("setting-turn-sound").uncheck();

    // Action reaches them (SB calls → BB's option) — no cue fires.
    await actingPage.getByRole("button", { name: /^CALL$/ }).click();
    await expect(waitingPage.getByRole("button", { name: ACTION_RX }).first()).toBeVisible({
      timeout: 30_000,
    });
    expect(await waitingPage.evaluate(() => (window as any).__cardcoreTurnCues || 0)).toBe(0);

    // The preference is device-local and survives a reload (which resumes
    // the game from the PDS).
    await waitingPage.reload();
    await expect(waitingPage.getByTestId("phase")).toBeVisible({ timeout: 30_000 });
    await waitingPage.getByTestId("settings-toggle").click();
    await expect(waitingPage.getByTestId("setting-turn-sound")).not.toBeChecked();

    await a.ctx.close();
    await b.ctx.close();
  });
});
