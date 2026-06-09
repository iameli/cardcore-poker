import { test, expect } from "@playwright/test";
import { demoSignIn, freshContext } from "./helpers";

/**
 * E2E auth-expiry test: when the PDS rejects our token (401 invalid_token,
 * e.g. an expired OAuth session), the app bounces the user to the sign-in
 * screen and — after they re-authenticate — returns them to where they were.
 */

test.describe("auth expiry (PDS-only)", () => {
  test("401 bounces to sign-in and returns the user afterwards", async ({ browser }) => {
    const a = await freshContext(browser); // host
    const b = await freshContext(browser); // joiner whose token "expires"

    await Promise.all([demoSignIn(a.page), demoSignIn(b.page)]);

    // A opens a room; B lands on its join page.
    await a.page.getByTestId("create-open-room").click();
    await expect(a.page.getByTestId("copy-table-uri")).toBeVisible({ timeout: 15_000 });
    const tid = (await a.page.getByTestId("copy-table-uri").locator("code").innerText())
      .trim()
      .split("/")
      .pop()!;
    const didA = await a.page.evaluate(
      () => JSON.parse(localStorage.getItem("cardcore_demo_session")!).did,
    );
    const tableUri = `at://${didA}/re.cardco.poker.table/${tid.trim()}`;
    await b.page.goto(`/${tableUri}`);
    await expect(b.page.getByTestId("request-join")).toBeVisible({ timeout: 15_000 });

    // B's PDS starts rejecting writes, like an expired OAuth token would.
    await b.page.route("**/xrpc/com.atproto.repo.putRecord", (route) =>
      route.fulfill({
        status: 401,
        contentType: "application/json",
        body: JSON.stringify({
          error: "invalid_token",
          message: '"exp" claim timestamp check failed',
        }),
      }),
    );

    // The failed write dumps B at the sign-in screen…
    await b.page.getByTestId("request-join").click();
    await expect(b.page.getByRole("button", { name: /Play in Demo Mode/i })).toBeVisible({
      timeout: 15_000,
    });
    await b.page.unroute("**/xrpc/com.atproto.repo.putRecord");

    // …and re-authenticating brings B straight back to the room (the demo
    // flow reuses the same stored account, so it's the same player).
    await b.page.getByRole("button", { name: /Play in Demo Mode/i }).click();
    await expect(b.page.getByTestId("request-join")).toBeVisible({ timeout: 15_000 });
    await expect(b.page).toHaveURL(/\/at:\/\//);

    // Variant: a real OAuth redirect lands on the redirect path, losing the
    // table path — the saved return-path restores it. Simulate by forcing
    // another 401 bounce, then reloading from "/".
    await b.page.route("**/xrpc/com.atproto.repo.putRecord", (route) =>
      route.fulfill({
        status: 401,
        contentType: "application/json",
        body: JSON.stringify({ error: "invalid_token", message: "expired" }),
      }),
    );
    await b.page.getByTestId("request-join").click();
    await expect(b.page.getByRole("button", { name: /Play in Demo Mode/i })).toBeVisible({
      timeout: 15_000,
    });
    await b.page.unroute("**/xrpc/com.atproto.repo.putRecord");

    await b.page.goto("/"); // path lost, sessionStorage survives
    // The stored demo session restores and the return-path routes B back.
    await expect(b.page.getByTestId("request-join")).toBeVisible({ timeout: 15_000 });
    await expect(b.page).toHaveURL(/\/at:\/\//);

    await a.ctx.close();
    await b.ctx.close();
  });
});
