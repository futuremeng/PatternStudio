import { expect, test } from "@playwright/test";

// Runs first (file order). Removes every registered project, asserts the empty
// state, then re-registers the seeded folder so the later specs have a project.
test("empty projects state", async ({ page, request }) => {
  const list = await request.get("/api/v1/projects");
  expect(list.ok()).toBeTruthy();
  const { projects } = await list.json();

  try {
    for (const p of projects) {
      const del = await request.delete(
        `/api/v1/projects/${encodeURIComponent(p.id)}`,
      );
      expect(del.status()).toBe(204);
    }

    const after = await request.get("/api/v1/projects");
    expect((await after.json()).projects).toEqual([]);

    await page.goto("/#/");
    const empty = page.locator(".empty-state");
    await expect(empty).toBeVisible();
    await expect(empty).toContainText("No projects yet");

    // Keyboard reachability smoke: one Tab must land focus on a control.
    await page.keyboard.press("Tab");
    const tag = await page.evaluate(
      () => document.activeElement?.tagName ?? "",
    );
    expect(["A", "BUTTON", "INPUT", "SELECT", "TEXTAREA"]).toContain(tag);
  } finally {
    // Restore whatever was removed so subsequent specs can find the project.
    for (const p of projects) {
      await request.post("/api/v1/projects", { data: { path: p.path } });
    }
  }
});
