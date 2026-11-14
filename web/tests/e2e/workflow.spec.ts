import { expect, test } from "@playwright/test";
import type { APIRequestContext } from "@playwright/test";

async function firstProjectId(request: APIRequestContext) {
  const res = await request.get("/api/v1/projects");
  expect(res.ok()).toBeTruthy();
  const { projects } = await res.json();
  expect(projects.length).toBeGreaterThan(0);
  return projects[0].id as string;
}

// Full happy path: projects list -> project detail -> image editor (auto
// trace) -> layer visibility toggles -> SVG download.
test("project -> images -> editor trace -> layer toggles -> download", async ({
  page,
  request,
}) => {
  const id = await firstProjectId(request);
  await page.goto("/#/");
  await page.locator(".project-card").first().click();
  await expect(page).toHaveURL(/#\/project\//);
  await expect(page.locator(".image-card")).toHaveCount(3);

  // Image cards navigate to the lineage tree, so open the editor by route.
  await page.goto(
    `/#/project/${encodeURIComponent(id)}/image/${encodeURIComponent("color.png")}`,
  );

  // The auto-trace (interactive vectorize) fills the preview; the Download
  // button enables only once the vector svg exists. A debug binary on 9 solid
  // colors stays in the seconds range; 90s is generous.
  await expect(page.getByRole("button", { name: "Download SVG" })).toBeEnabled({ timeout: 90_000 });

  // Layering is manual by design: Process applies the layer strategy to the
  // cached document and fills the layer panel.
  await page.getByRole("button", { name: /Process/ }).click();
  const layer = page.locator(".layer-card").first();
  await expect(layer).toBeVisible({ timeout: 90_000 });

  const checkbox = layer.locator('input[type="checkbox"]');
  const initiallyChecked = await checkbox.isChecked();
  await layer.click(); // card click toggles layer visibility
  expect(await checkbox.isChecked()).toBe(!initiallyChecked);
  await layer.click();
  expect(await checkbox.isChecked()).toBe(initiallyChecked);

  const [download] = await Promise.all([
    page.waitForEvent("download"),
    page.getByRole("button", { name: "Download SVG" }).click(),
  ]);
  expect(download.suggestedFilename().toLowerCase()).toMatch(/\.svg$/);
});
