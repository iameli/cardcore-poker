import { test, expect } from "@playwright/test";

test("SignIn is shown on first load", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: /CARDCORE POKER/i })).toBeVisible();
  await expect(page.getByRole("button", { name: /Play in Demo Mode/i })).toBeVisible();
});

test("a returning user reloads into the lobby without a sign-in flash", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: /Play in Demo Mode/i }).click();
  await expect(page.getByRole("heading", { name: /^Lobby$/i })).toBeVisible({ timeout: 15_000 });

  // On reload the app must show a loading state until the session resolves —
  // if the sign-in form renders first (the old behavior), it wins this race.
  await page.reload();
  const winner = await Promise.race([
    page
      .getByRole("heading", { name: /^Lobby$/i })
      .waitFor({ timeout: 15_000 })
      .then(() => "lobby"),
    page
      .getByRole("button", { name: /Play in Demo Mode/i })
      .waitFor({ timeout: 15_000 })
      .then(() => "signin"),
  ]);
  expect(winner).toBe("lobby");
});
