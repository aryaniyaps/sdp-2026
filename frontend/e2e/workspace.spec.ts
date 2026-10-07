import { expect, test } from "@playwright/test";

const factId = "11111111-1111-4111-8111-111111111111";
const snapshot = {
  entities: [{ id: "entity", name: "Ada" }],
  assertions: [
    {
      id: factId,
      statement: "Ada uses Rust",
      kind: "fact",
      status: "active",
      confidence: 0.95,
      valid_from: "2026-01-01T00:00:00Z",
      recorded_at: "2026-01-01T00:00:00Z",
      valid_to: null,
    },
  ],
  edges: [],
  status: {
    ready: true,
    outstanding_jobs: 0,
    stale_observations: 0,
    last_projection_completed_at: null,
    jobs: [],
  },
};

const projection = {
  nodes: [
    {
      id: factId,
      kind: "assertion",
      name: "Ada uses Rust",
      statement: "Ada uses Rust",
      status: "active",
      assertion_kind: "fact",
    },
    { id: "entity", kind: "entity", name: "Ada", entity_type: "person" },
  ],
  relationships: [],
  counts: { nodes: 2, relationships: 0 },
  truncated: false,
};

const memories = {
  memories: [],
  counts: {
    memories: 0,
    supported: 0,
    assertions: 1,
    assertions_by_status: { active: 1 },
    jobs: {},
  },
  truncated: false,
};

const trace = {
  id: "trace-1",
  operation_type: "recall_v2",
  status: "succeeded",
  duration_ms: 20,
  started_at: "2026-01-01T00:00:00Z",
  request: {},
  steps: [
    {
      ordinal: 0,
      stage: "lexical_retrieval",
      status: "ok",
      duration_ms: 3,
      details: { matches: 1 },
    },
  ],
};

test.beforeEach(async ({ page }) => {
  await page.route("**/healthz", (route) =>
    route.fulfill({
      json: { database: true, embedder: true, worker_model: "pi/test/model" },
    }),
  );
  await page.route("**/api/v2/**", (route) => {
    const url = new URL(route.request().url());
    const body = url.pathname.endsWith("/projection")
      ? projection
      : url.pathname.endsWith("/memories")
        ? memories
        : url.pathname.endsWith("/graph")
          ? snapshot
          : url.pathname.endsWith("/traces")
            ? [trace]
            : url.pathname.includes("/assertions/")
              ? {
                  statement: "Ada uses Rust",
                  sources: [{ quote: "I use Rust" }],
                }
              : url.pathname.endsWith("/retain")
                ? { job_id: "job-1" }
                : {
                    context: "Ada uses Rust",
                    estimated_tokens: 4,
                    elapsed_ms: 20,
                    trace_id: trace.id,
                    degraded_reasons: [],
                    ranking: [],
                  };
    return route.fulfill({ json: body });
  });
});

test("retains evidence, recalls it, and shows provenance and traces", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto("/");
  await expect(page.getByText("Database connected")).toBeVisible();
  await page.getByLabel("Interaction text").fill("I use Rust");
  const saved = page.waitForRequest((request) =>
    request.url().endsWith("/retain"),
  );
  await page.getByRole("button", { name: "Retain evidence" }).click();
  expect((await saved).postDataJSON().events[0].content).toBe("I use Rust");
  await expect(page.getByText(/Evidence saved/)).toBeVisible();
  await page.getByRole("button", { name: "Recall evidence" }).click();
  await expect(
    page.locator("pre").filter({ hasText: "Ada uses Rust" }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Inspect evidence for Ada uses Rust" })
    .click();
  await expect(page.getByRole("dialog")).toContainText("I use Rust");
  await page.getByRole("button", { name: "Close" }).click();
  await expect(page.getByText("1. lexical_retrieval")).toBeVisible();
  expect(errors).toEqual([]);
});

test("loads another namespace and uses it for subsequent requests", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByLabel("Namespace", { exact: true }).fill("user:student");
  await page.getByRole("button", { name: "Load", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Open graph" }),
  ).toHaveAttribute("href", "/graph?namespace=user%3Astudent");
  const recall = page.waitForRequest((request) =>
    request.url().endsWith("/recall"),
  );
  await page.getByRole("button", { name: "Recall evidence" }).click();
  expect((await recall).postDataJSON().namespace).toBe("user:student");
});

test("shows server errors and allows retry", async ({ page }) => {
  await page.route("**/api/v2/retain", (route) =>
    route.fulfill({
      status: 503,
      json: { error: "Database temporarily unavailable" },
    }),
  );
  await page.goto("/");
  await page.getByRole("button", { name: "Retain evidence" }).click();
  await expect(page.getByRole("alert")).toContainText(
    "Database temporarily unavailable",
  );
  await expect(
    page.getByRole("button", { name: "Retain evidence" }),
  ).toBeEnabled();
});

test("dashboard fits a phone screen", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/");
  await expect(
    page.getByRole("heading", { name: "Memory workspace" }),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
});

test("graph loads nodes and supports search and layer toggles", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto("/graph?namespace=review&poll=0");
  await expect(page.getByText("review", { exact: true })).toBeVisible();
  await expect(page.locator("#c-fact")).toHaveText("1");
  await page.getByLabel("Search nodes").fill("Rust");
  await expect(page.locator("#matches")).toContainText("1");
  await page.locator("#t-fact").click();
  await expect(page.locator("#t-fact")).toHaveAttribute(
    "aria-pressed",
    "false",
  );
  expect(errors).toEqual([]);
});

test("embedded graph hides actions unavailable through the demo proxy", async ({
  page,
}) => {
  await page.goto("/graph?namespace=review&poll=0&readonly=1");
  await expect(page.locator("#c-fact")).toHaveText("1");
  await expect(page.locator("#clear")).toBeHidden();
  await expect(page.getByRole("link", { name: "Workspace" })).toBeHidden();
});
