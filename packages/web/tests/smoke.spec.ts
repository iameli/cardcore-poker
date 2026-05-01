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
