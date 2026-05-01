import { test, expect } from "@playwright/test";

test("dev server serves SignIn page", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: /CARDCORE POKER/i })).toBeVisible();
  await expect(page.getByRole("button", { name: /Play in Demo Mode/i })).toBeVisible();
});
