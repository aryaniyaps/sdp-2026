import { expect, test } from "@playwright/test";

// Routing tests use a stateful API fixture; live Pi/graph acceptance lives in dashboard-live.spec.ts.
test.beforeEach(async ({ page }) => {
  const projects = ["payments-api", "mobile-app", "scratch"];
  const sessions = new Map<string, any>();
  await page.route("**/api/v2/projects", async (route) => {
    if (route.request().method() === "POST") {
      const { id } = route.request().postDataJSON();
      if (!projects.includes(id)) projects.push(id);
      return route.fulfill({ json: { id } });
    }
    return route.fulfill({ json: { projects } });
  });
  await page.route("**/api/v2/dashboard/sessions**", async (route) => {
    const request = route.request();
    const id = new URL(request.url()).pathname.split("/")[5];
    if (!id) {
      const input = request.postDataJSON();
      const session = {
        id: crypto.randomUUID(),
        project: input.project,
        namespace: input.namespace || `project:${input.project}`,
        revision: 0,
        applied_revision: 0,
      };
      sessions.set(session.id, session);
      return route.fulfill({ json: session });
    }
    const session = sessions.get(id);
    if (request.method() === "POST") {
      session.namespace = request.postDataJSON().namespace;
      session.revision++;
      session.applied_revision = session.revision;
    }
    return route.fulfill({ json: session });
  });
  await page.route("**/term/**", (route) =>
    route.fulfill({ contentType: "text/html", body: "<p>Pi terminal</p>" }),
  );
  await page.route("**/graph?*", (route) =>
    route.fulfill({ contentType: "text/html", body: "<p>Memory graph</p>" }),
  );
});

test("opens the Pi console and graph at the root URL", async ({ page }) => {
  await page.goto("/");
  await expect(
    page.getByRole("heading", { name: "Pi with shared memory" }),
  ).toBeVisible();
  await expect(page.locator("#term")).toHaveAttribute(
    "src",
    /arg=payments-api&arg=project%3Apayments-api&arg=/,
  );
  await expect(page.locator("#graph")).toHaveAttribute(
    "src",
    "/graph?namespace=project%3Apayments-api&readonly=1",
  );
});

test("switches projects with isolated defaults and opens new sessions", async ({
  page,
}) => {
  await page.goto("/demo");
  await page.getByRole("button", { name: "Session in mobile-app" }).click();
  await expect(page.locator("#term")).toHaveAttribute(
    "src",
    /arg=mobile-app&arg=project%3Amobile-app&arg=/,
  );
  await expect(page.locator("#graph")).toHaveAttribute(
    "src",
    "/graph?namespace=project%3Amobile-app&readonly=1",
  );
  const source = await page.locator("#term").getAttribute("src");
  await page.getByRole("button", { name: "New session", exact: true }).click();
  await expect(page.locator("#term")).not.toHaveAttribute("src", source!);
});

test("changes namespace without restarting Pi and retains it on reload", async ({
  page,
}) => {
  await page.goto("/demo");
  await expect(page.locator("#term")).toBeVisible();
  const source = await page.locator("#term").getAttribute("src");
  await page.getByLabel("Namespace", { exact: true }).fill("project:a & b");
  await page.getByRole("button", { name: "Apply", exact: true }).click();
  await expect(page.locator("#graph")).toHaveAttribute(
    "src",
    "/graph?namespace=project%3Aa+%26+b&readonly=1",
  );
  await expect(page.locator("#term")).toHaveAttribute("src", source!);
  await page.reload();
  await expect(page.getByLabel("Namespace", { exact: true })).toHaveValue(
    "project:a & b",
  );
});

test("creates a project and retains it on reload", async ({ page }) => {
  await page.goto("/demo");
  await expect(page.locator("#term")).toBeVisible();
  await page.getByLabel("New project", { exact: true }).fill("inventory-api");
  await page.getByRole("button", { name: "Create project" }).click();
  await expect(page.getByLabel("Namespace", { exact: true })).toHaveValue(
    "project:inventory-api",
  );
  await expect(page.locator("#term")).toHaveAttribute(
    "src",
    /arg=inventory-api&arg=project%3Ainventory-api&arg=/,
  );
  await page.reload();
  await expect(
    page.getByRole("button", { name: "Session in inventory-api" }),
  ).toHaveAttribute("aria-pressed", "true");
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

test("clear requires exact confirmation, supports cancellation, and preserves Pi", async ({
  page,
}) => {
  const cleared: unknown[] = [];
  await page.route("**/api/v2/graph/clear", async (route) => {
    cleared.push(route.request().postDataJSON());
    await route.fulfill({ json: { graph: { cleared: true } } });
  });
  await page.goto("/demo");
  await expect(page.locator("#term")).toBeVisible();
  const terminal = await page.locator("#term").getAttribute("src");
  await page
    .getByRole("button", { name: "Clear namespace", exact: true })
    .click();
  await page.getByLabel("Type the namespace to confirm").fill("wrong");
  await expect(
    page.getByRole("button", { name: "Delete namespace memory" }),
  ).toBeDisabled();
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  expect(cleared).toEqual([]);
  await page
    .getByRole("button", { name: "Clear namespace", exact: true })
    .click();
  await page
    .getByLabel("Type the namespace to confirm")
    .fill("project:payments-api");
  await page.getByRole("button", { name: "Delete namespace memory" }).click();
  await expect(
    page.getByText("Cleared project:payments-api.", { exact: true }),
  ).toBeVisible();
  expect(cleared).toEqual([
    { namespace: "project:payments-api", confirm: "project:payments-api" },
  ]);
  await expect(page.locator("#term")).toHaveAttribute("src", terminal!);
});

test("clear failures stay visible and allow retry without claiming success", async ({
  page,
}) => {
  let attempts = 0;
  await page.route("**/api/v2/graph/clear", async (route) => {
    attempts++;
    await route.fulfill(
      attempts === 1
        ? {
            status: 409,
            json: { error: "Worker is running; retry when finished" },
          }
        : { json: { graph: { cleared: false } } },
    );
  });
  await page.goto("/demo");
  await expect(page.locator("#term")).toBeVisible();
  await page
    .getByRole("button", { name: "Clear namespace", exact: true })
    .click();
  await page
    .getByLabel("Type the namespace to confirm")
    .fill("project:payments-api");
  await page.getByRole("button", { name: "Delete namespace memory" }).click();
  await expect(page.getByRole("dialog").getByRole("alert")).toContainText(
    "Worker is running",
  );
  await expect(
    page.getByText("Cleared project:payments-api.", { exact: true }),
  ).not.toBeVisible();
  await page.getByRole("button", { name: "Delete namespace memory" }).click();
  await expect(page.getByText(/Graph cleanup is queued/)).toBeVisible();
});

test("shows starter projects immediately while the live catalog loads", async ({
  page,
}) => {
  let release!: () => void;
  const loading = new Promise<void>((resolve) => {
    release = resolve;
  });
  await page.route("**/api/v2/projects", async (route) => {
    await loading;
    await route.fulfill({
      json: {
        projects: ["payments-api", "mobile-app", "scratch", "custom-project"],
      },
    });
  });
  await page.goto("/demo", { waitUntil: "domcontentloaded" });
  for (const name of ["payments-api", "mobile-app", "scratch"]) {
    const button = page.getByRole("button", {
      name: `Session in ${name}`,
      exact: true,
    });
    await expect(button).toBeVisible();
    await expect(button).toBeDisabled();
  }
  release();
  await expect(
    page.getByRole("button", { name: "Session in custom-project" }),
  ).toBeEnabled();
  await expect(
    page.getByRole("button", { name: "Session in payments-api" }),
  ).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByLabel("Namespace", { exact: true })).toHaveValue(
    "project:payments-api",
  );
});
