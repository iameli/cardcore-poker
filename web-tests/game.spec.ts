import { test, expect } from "@playwright/test";

test.describe("Cardcore Poker", () => {
  test("loads WASM and displays table", async ({ page }) => {
    await page.goto("/");

    // Wait for WASM to load (loading screen disappears)
    await expect(page.getByTestId("app")).toBeVisible({ timeout: 15000 });

    // Poker table should be rendered
    await expect(page.getByTestId("poker-table")).toBeVisible();

    // Should have player seats
    await expect(page.getByTestId("player-seat-0")).toBeVisible();
    await expect(page.getByTestId("player-seat-1")).toBeVisible();
    await expect(page.getByTestId("player-seat-2")).toBeVisible();
  });

  test("step through a full game", async ({ page }) => {
    await page.goto("/");
    await expect(page.getByTestId("app")).toBeVisible({ timeout: 15000 });

    // Jump to end
    await page.getByTestId("btn-end").click();

    // Should have community cards after stepping to end
    await expect(page.getByTestId("community-cards")).toBeVisible();

    // Event log should have entries
    await expect(page.getByTestId("event-log")).toBeVisible();
  });

  test("auto-play plays through entire game", async ({ page }) => {
    await page.goto("/");
    await expect(page.getByTestId("app")).toBeVisible({ timeout: 15000 });

    // Click play button
    await page.getByTestId("btn-play").click();

    // Wait for game to finish auto-playing (events will reach the end)
    // Community cards should appear eventually
    await expect(page.getByTestId("community-cards")).toBeVisible({
      timeout: 30000,
    });
  });

  test("switch between 2, 3, 4 players", async ({ page }) => {
    await page.goto("/");
    await expect(page.getByTestId("app")).toBeVisible({ timeout: 15000 });

    // Switch to 2 players
    await page.getByTestId("btn-2p").click();
    await expect(page.getByTestId("player-seat-0")).toBeVisible();
    await expect(page.getByTestId("player-seat-1")).toBeVisible();

    // Switch to 4 players
    await page.getByTestId("btn-4p").click();
    // Wait for WASM to re-simulate
    await page.waitForTimeout(1000);
    await expect(page.getByTestId("player-seat-3")).toBeVisible();
  });

  test("new game produces different cards", async ({ page }) => {
    await page.goto("/");
    await expect(page.getByTestId("app")).toBeVisible({ timeout: 15000 });

    // Jump to end of first game
    await page.getByTestId("btn-end").click();
    const firstLog = await page.getByTestId("event-log").textContent();

    // Start a new game
    await page.getByTestId("btn-new").click();
    await page.waitForTimeout(1000);
    await page.getByTestId("btn-end").click();
    const secondLog = await page.getByTestId("event-log").textContent();

    // Different games should have different event logs
    expect(firstLog).not.toBe(secondLog);
  });

  test("event log shows all game phases", async ({ page }) => {
    await page.goto("/");
    await expect(page.getByTestId("app")).toBeVisible({ timeout: 15000 });

    // Jump to end
    await page.getByTestId("btn-end").click();

    const logText = await page.getByTestId("event-log").textContent();

    // Should contain key phases — seeds are always verified even on fold wins
    expect(logText).toContain("Table:");
    expect(logText).toContain("dealt");
    expect(logText).toContain("Seeds verified");
  });
});
