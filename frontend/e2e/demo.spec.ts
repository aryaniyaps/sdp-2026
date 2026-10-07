import { expect, test } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.route("**/term/**", (route) =>
    route.fulfill({ contentType: "text/html", body: "<p>Pi terminal</p>" }),
  );
  await page.route("**/graph?*", (route) =>
    route.fulfill({ contentType: "text/html", body: "<p>Memory graph</p>" }),
  );
});

test("opens fresh Pi sessions in different directories with shared graph memory", async ({
  page,
}) => {
  await page.goto("/demo");
  const terminal = page.locator("#term");
  await expect(terminal).toHaveAttribute(
    "src",
    "/term/?arg=payments-api&arg=user%3Adev&session=0",
  );
  const graphSource = await page.locator("#graph").getAttribute("src");
  await page.getByRole("button", { name: "Session in mobile-app" }).click();
  await expect(terminal).toHaveAttribute(
    "src",
    "/term/?arg=mobile-app&arg=user%3Adev&session=1",
  );
  await expect(page.locator("#graph")).toHaveAttribute("src", graphSource!);
  await page.getByRole("button", { name: "New session", exact: true }).click();
  await expect(terminal).toHaveAttribute(
    "src",
    "/term/?arg=mobile-app&arg=user%3Adev&session=2",
  );
});

test("applies the same encoded namespace to Pi and the graph and retains it on reload", async ({
  page,
}) => {
  await page.goto("/demo");
  await page.getByLabel("Namespace", { exact: true }).fill("project:a & b");
  await page.getByRole("button", { name: "Apply", exact: true }).click();
  const terminal = new URL(
    (await page.locator("#term").getAttribute("src"))!,
    page.url(),
  );
  const graph = new URL(
    (await page.locator("#graph").getAttribute("src"))!,
    page.url(),
  );
  expect(terminal.searchParams.getAll("arg")).toEqual([
    "payments-api",
    "project:a & b",
  ]);
  expect(graph.searchParams.get("namespace")).toBe("project:a & b");
  await page.reload();
  await expect(page.getByLabel("Namespace", { exact: true })).toHaveValue(
    "project:a & b",
  );
});

test("keeps both panes visible on mobile without horizontal overflow", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/demo");
  await expect(page.locator("#term")).toBeVisible();
  await expect(page.locator("#graph")).toBeVisible();
  expect(
    await page.evaluate(() => document.documentElement.scrollWidth),
  ).toBeLessThanOrEqual(390);
});
