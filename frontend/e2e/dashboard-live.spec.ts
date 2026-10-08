import { expect, test } from "@playwright/test";
import { spawn } from "node:child_process";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve, join } from "node:path";
import { createInterface } from "node:readline";

test("live Pi namespace and clear commands synchronize with the dashboard graph", async ({
  page,
  request,
}) => {
  test.skip(
    !process.env.LIVE_DASHBOARD,
    "Requires isolated memory service, Neo4j and Pi",
  );
  test.setTimeout(180_000);
  const base = process.env.MEMORY_API_URL!;
  const project = `sync-${crypto.randomUUID().slice(0, 8)}`;
  const ns = `project:${project}`,
    other = `${ns}-other`;
  const root = await mkdtemp(join(tmpdir(), "dashboard-pi-"));
  let pi: ReturnType<typeof spawn> | undefined;
  const events: any[] = [];
  let stderr = "",
    confirmation = "";
  const send = (value: unknown) =>
    pi!.stdin!.write(JSON.stringify(value) + "\n");
  const command = (message: string) =>
    send({ id: crypto.randomUUID(), type: "prompt", message });
  try {
    // Real Pi below uses the exact terminal binding; only ttyd's rendering is substituted.
    await page.route("**/term/**", (route) =>
      route.fulfill({
        contentType: "text/html",
        body: "<p>Pi RPC acceptance session</p>",
      }),
    );
    await page.goto("/demo");
    await expect(page.locator("#term")).toBeVisible();
    await page.getByLabel("New project", { exact: true }).fill(project);
    await page.getByRole("button", { name: "Create project" }).click();
    await expect(page.getByLabel("Namespace", { exact: true })).toHaveValue(ns);
    const source = (await page.locator("#term").getAttribute("src"))!;
    const args = new URL(source, page.url()).searchParams.getAll("arg");
    expect(args.slice(0, 2)).toEqual([project, ns]);
    pi = spawn(
      "pi",
      [
        "--mode",
        "rpc",
        "--no-extensions",
        "--no-skills",
        "--no-prompt-templates",
        "--no-context-files",
        "--no-builtin-tools",
        "-e",
        resolve("../integrations/pi/extension.ts"),
        "--session-dir",
        join(root, "sessions"),
      ],
      {
        cwd: root,
        env: {
          ...process.env,
          MEMORY_URL: base,
          MEMORY_NAMESPACE: ns,
          MEMORY_DASHBOARD_SESSION: args[2],
          MEMORY_SPOOL: join(root, "spool"),
          PI_OFFLINE: "1",
        },
        stdio: ["pipe", "pipe", "pipe"],
      },
    );
    createInterface({ input: pi.stdout! }).on("line", (line) => {
      const event = JSON.parse(line);
      events.push(event);
      if (event.type === "extension_ui_request" && event.method === "input")
        send({
          type: "extension_ui_response",
          id: event.id,
          value: confirmation,
        });
    });
    pi.stderr!.on("data", (data) => {
      stderr += data.toString();
    });
    await expect(
      page.getByText("Pi namespace synced", { exact: true }),
    ).toBeVisible({ timeout: 20_000 });
    await page.getByLabel("Namespace", { exact: true }).fill(other);
    await page.getByRole("button", { name: "Apply", exact: true }).click();
    await expect(page.locator("#graph")).toHaveAttribute(
      "src",
      `/graph?namespace=${encodeURIComponent(other)}&readonly=1`,
    );
    await expect(
      page.getByText("Pi namespace synced", { exact: true }),
    ).toBeVisible();
    await expect(page.locator("#term")).toHaveAttribute("src", source);
    await expect
      .poll(() =>
        events.some(
          (event) =>
            event.method === "setStatus" &&
            event.statusKey === "memory-namespace" &&
            event.statusText === `Namespace: ${other}`,
        ),
      )
      .toBe(true);
    command("/memory-namespace " + other);
    await expect
      .poll(() =>
        events.some(
          (e) => e.message === `Memory namespace is already ${other}`,
        ),
      )
      .toBe(true);
    command("/memory-namespace " + ns);
    await expect(page.getByLabel("Namespace", { exact: true })).toHaveValue(ns);
    await expect(page.locator("#graph")).toHaveAttribute(
      "src",
      `/graph?namespace=${encodeURIComponent(ns)}&readonly=1`,
    );
    await expect(page.locator("#term")).toHaveAttribute("src", source);
    for (const namespace of [ns, other]) {
      const response = await request.post(`${base}/api/v2/retain`, {
        data: {
          namespace,
          session_id: project,
          external_id: namespace,
          events: [
            {
              role: "user",
              content: `${project} uses PostgreSQL for storage.`,
              occurred_at: new Date().toISOString(),
            },
          ],
        },
      });
      expect(response.ok(), await response.text()).toBe(true);
    }
    const graph = page.frameLocator("#graph");
    await expect(graph.locator("#strip")).toContainText("1", {
      timeout: 20_000,
    });
    confirmation = "wrong-namespace";
    command("/memory-clear");
    await expect
      .poll(() =>
        events.some((e) => e.message?.includes("nothing was cleared")),
      )
      .toBe(true);
    let memories = await (
      await request.get(
        `${base}/api/v2/graph/memories?namespace=${encodeURIComponent(ns)}`,
      )
    ).json();
    expect(memories.memories.length).toBeGreaterThan(0);
    confirmation = ns;
    for (let attempt = 0; attempt < 20; attempt++) {
      const offset = events.length;
      command("/memory-clear");
      await expect
        .poll(
          () =>
            events
              .slice(offset)
              .some((e) =>
                /Cleared namespace|Could not clear/.test(e.message || ""),
              ),
          { timeout: 20_000 },
        )
        .toBe(true);
      if (
        events
          .slice(offset)
          .some((e) => e.message?.startsWith("Cleared namespace"))
      )
        break;
      // Retry only a reported active-worker conflict, after giving the lease time to finish.
      expect(
        events
          .slice(offset)
          .find((e) => e.message?.startsWith("Could not clear"))?.message,
      ).toContain("409");
      await page.waitForTimeout(2000);
    }
    await expect(graph.locator("#strip")).toContainText(
      "Postgres: 0 memories",
      { timeout: 15_000 },
    );
    await expect(graph.locator("#msg")).toHaveClass(/show/);
    await expect(graph.locator("#msgtext")).toContainText(
      `No memories yet for ${ns}`,
    );
    memories = await (
      await request.get(
        `${base}/api/v2/graph/memories?namespace=${encodeURIComponent(other)}`,
      )
    ).json();
    expect(memories.memories.length).toBeGreaterThan(0);
    command("/memory-status");
    await expect
      .poll(() => events.some((e) => e.statusText?.includes('"pending":0')))
      .toBe(true);
    await expect(graph.locator("#msgtext")).toContainText(
      `No memories yet for ${ns}`,
    );
    // The dashboard button clears the other populated namespace through the real API.
    await page.getByLabel("Namespace", { exact: true }).fill(other);
    await page.getByRole("button", { name: "Apply", exact: true }).click();
    await expect(page.locator("#graph")).toHaveAttribute(
      "src",
      `/graph?namespace=${encodeURIComponent(other)}&readonly=1`,
    );
    await expect(
      page.getByText("Pi namespace synced", { exact: true }),
    ).toBeVisible();
    await page
      .getByRole("button", { name: "Clear namespace", exact: true })
      .click();
    await page.getByLabel("Type the namespace to confirm").fill(other);
    await page.getByRole("button", { name: "Delete namespace memory" }).click();
    await expect(
      page.getByText(`Cleared ${other}.`, { exact: true }),
    ).toBeVisible({ timeout: 15_000 });
    await expect(graph.locator("#strip")).toContainText(
      "Postgres: 0 memories",
      { timeout: 15_000 },
    );
    await expect(graph.locator("#msg")).toHaveClass(/show/);
    await expect(page.locator("#term")).toHaveAttribute("src", source);
    await page.screenshot({
      path: "test-results/dashboard-live.png",
      fullPage: true,
    });
    await page.reload();
    await expect(
      page.getByRole("button", { name: `Session in ${project}` }),
    ).toHaveAttribute("aria-pressed", "true");
  } finally {
    if (pi && pi.exitCode === null && pi.signalCode === null) {
      const exited = new Promise<void>((resolve) =>
        pi!.once("exit", () => resolve()),
      );
      pi.stdin!.end();
      pi.kill("SIGTERM");
      const timeout = setTimeout(() => pi!.kill("SIGKILL"), 3000);
      await exited;
      clearTimeout(timeout);
    }
    await rm(root, { recursive: true, force: true });
    if (stderr) console.log(stderr.slice(-2000));
    console.log(
      JSON.stringify(
        events
          .filter(
            (e) => e.type === "extension_ui_request" || e.success === false,
          )
          .slice(-12),
      ),
    );
  }
});
