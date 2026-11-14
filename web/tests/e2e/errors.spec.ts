import { expect, test } from "@playwright/test";
import type { APIRequestContext } from "@playwright/test";

async function firstProjectId(request: APIRequestContext) {
  const res = await request.get("/api/v1/projects");
  expect(res.ok()).toBeTruthy();
  const { projects } = await res.json();
  expect(projects.length).toBeGreaterThan(0);
  return projects[0].id as string;
}

test("unreadable image surfaces a classified error banner", async ({
  page,
  request,
}) => {
  const id = await firstProjectId(request);
  await page.goto(
    `/#/project/${encodeURIComponent(id)}/image/${encodeURIComponent("junk.png")}`,
  );
  const banner = page.locator(".error.preview-error");
  await expect(banner).toBeVisible({ timeout: 60_000 });
  // EngineError code invalid_input -> error.invalidInput ("Invalid image: ...")
  await expect(banner).toContainText("Invalid image:");
});

// Timing-based: the seeded photo.png is per-pixel noise, so tracing it with a
// debug binary takes long enough that the second background request reliably
// lands while the first is still registered (register() answers Busy then).
test("concurrent background trace on one image answers 409 busy", async ({
  request,
}) => {
  const id = await firstProjectId(request);
  const body = {
    project_id: id,
    filename: "photo.png",
    priority: "background",
    request_id: "e2e-busy-1",
  };
  const first = request.post("/api/v1/trace", { data: body });
  // Give the server a moment to register the first trace before competing.
  await new Promise((r) => setTimeout(r, 500));
  const second = await request.post("/api/v1/trace", {
    data: { ...body, request_id: "e2e-busy-2" },
  });
  expect(second.status()).toBe(409);
  expect(await second.json()).toMatchObject({ busy: true, code: "busy" });

  const firstRes = await first;
  expect(firstRes.status()).toBe(200);
});
