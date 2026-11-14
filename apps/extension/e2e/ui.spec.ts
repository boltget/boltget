// The popup and the options page against the stub host: each connection
// state, the capture switch, Test connection and editing the rules.
import { EXTENSION_ID, eventually, expect, test } from "./fixtures";

const popup = `chrome-extension://${EXTENSION_ID}/popup.html`;
const options = `chrome-extension://${EXTENSION_ID}/options.html`;

test("the popup shows the connected app and switches capture off and on", async ({
  context,
  worker,
}) => {
  const page = await context.newPage();
  await page.goto(popup);
  await expect(page.getByRole("status")).toContainText("Connected to Boltget 0.3.0");

  const toggle = page.getByRole("switch");
  await expect(toggle).toBeChecked();
  await toggle.click({ force: true });
  await eventually(() =>
    worker.evaluate(async () => (await chrome.storage.local.get("captureOn")).captureOn === false),
  );
  await expect(page.getByText("Every download stays in the browser.")).toBeVisible();
});

test.describe("when the app is not running", () => {
  test.use({ stubMode: { app: null } });

  test("the popup says it starts when needed", async ({ context }) => {
    const page = await context.newPage();
    await page.goto(popup);
    await expect(page.getByRole("status")).toContainText("Boltget isn’t running");
  });
});

test("the popup offers the download page when Boltget is not installed", async ({
  context,
  stub,
}) => {
  stub.uninstall();
  const page = await context.newPage();
  await page.goto(popup);
  await expect(page.getByRole("status")).toContainText("Boltget isn’t installed");
  await expect(page.getByRole("link", { name: "Get Boltget" })).toHaveAttribute(
    "href",
    "https://boltget.app/download",
  );
});

test("the pause holds capture for ten minutes", async ({ context, worker }) => {
  const page = await context.newPage();
  await page.goto(popup);
  await page.getByRole("button", { name: "Pause for 10 minutes" }).click();
  const until = await eventually(() =>
    worker.evaluate(async () => (await chrome.storage.local.get("pausedUntil")).pausedUntil),
  );
  expect(Number(until) - Date.now()).toBeGreaterThan(9 * 60 * 1000);
  await expect(page.getByRole("button", { name: "Resume now" })).toBeVisible();
});

test("options: Test connection and editing the rules", async ({ context, worker, stub }) => {
  const page = await context.newPage();
  await page.goto(`${options}#welcome`);
  await expect(page.getByRole("heading", { name: "Boltget is ready" })).toBeVisible();
  await expect(page.getByTestId("access-state")).toHaveText("Allowed on all sites.");
  await expect(page.getByRole("status")).toContainText("Connected to Boltget 0.3.0");

  const before = stub.messages("hello").length;
  await page.getByRole("button", { name: "Test connection" }).click();
  await eventually(() => stub.messages("hello").length > before);

  await page.getByLabel("Smallest file").fill("16");
  await page.getByLabel("Smallest file").blur();
  await page.getByLabel("Never send from").fill("intranet.example\nhttps://Bank.example/login");
  await page.getByLabel("Never send from").blur();
  const rules = await eventually(() =>
    worker.evaluate(async () => {
      const { rules, rulesEdited } = await chrome.storage.local.get(["rules", "rulesEdited"]);
      const r = rules as { min_size: number; never_sites: string[] };
      return rulesEdited && r.never_sites.length === 2 ? r : null;
    }),
  );
  expect(rules.min_size).toBe(16 * 1024 * 1024);
  expect(rules.never_sites).toEqual(["intranet.example", "bank.example"]);

  await page.getByRole("button", { name: "Use Boltget’s rules" }).click();
  await eventually(() =>
    worker.evaluate(
      async () => (await chrome.storage.local.get("rulesEdited")).rulesEdited === false,
    ),
  );
});
