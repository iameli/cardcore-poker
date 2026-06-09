import { test, expect } from "@playwright/test";

test("password gate appears on first load", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("gate-password")).toBeVisible();
  // SignIn UI is hidden until the gate is unlocked.
  await expect(page.getByRole("button", { name: /Play in Demo Mode/i })).toBeHidden();

  await page.getByTestId("gate-password").fill("pocketrockets");
  await page.getByTestId("gate-submit").click();

  await expect(page.getByRole("heading", { name: /CARDCORE POKER/i })).toBeVisible();
  await expect(page.getByRole("button", { name: /Play in Demo Mode/i })).toBeVisible();
});

test("once unlocked, SignIn is shown directly", async ({ context, page }) => {
  await context.addInitScript(() => localStorage.setItem("cardcore_unlocked", "1"));
  await page.goto("/");
  await expect(page.getByRole("heading", { name: /CARDCORE POKER/i })).toBeVisible();
  await expect(page.getByRole("button", { name: /Play in Demo Mode/i })).toBeVisible();
});

test("a returning user reloads into the lobby without a sign-in flash", async ({
  context,
  page,
}) => {
  await context.addInitScript(() => localStorage.setItem("cardcore_unlocked", "1"));
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
