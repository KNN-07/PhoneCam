import { expect, test } from "@playwright/test";

test.beforeEach(async ({ context }) => {
  await context.addInitScript(() => {
    window.__phonecamCalls = [];
    window.__phonecamStatus = {
      connected: false,
      state: "listening",
      last_error: null
    };
    window.__TAURI__ = {
      core: {
        invoke: async (command, args = {}) => {
          window.__phonecamCalls.push({ command, args });
          switch (command) {
            case "get_status":
              return window.__phonecamStatus;
            case "generate_qr_code":
              return '<svg role="img" aria-label="PhoneCam QR"></svg>';
            case "get_qr_connection_uris":
              return ["phonecam://192.0.2.10:7878?name=PhoneCam%20Desktop"];
            default:
              return null;
          }
        }
      }
    };
  });
});

test("renders all v1 stream presets and listener state", async ({ page }) => {
  await page.goto("/");

  await expect(page).toHaveTitle("PhoneCam");
  await expect(page.locator("#resolution-select option")).toHaveCount(3);
  await expect(page.locator("#fps-select option")).toHaveCount(3);
  await expect(page.locator("#status-indicator .status-text")).toHaveText("listening");
  await expect(page.locator("#switch-camera-btn")).toBeDisabled();
});

test("sends and persists stream settings for a connected phone", async ({ page }) => {
  await page.goto("/");
  await page.evaluate(() => {
    window.__phonecamStatus = {
      connected: true,
      state: "connected",
      last_error: null
    };
  });

  await expect(page.locator("#switch-camera-btn")).toBeEnabled({ timeout: 2_000 });
  await page.selectOption("#resolution-select", "1920x1080");
  await page.selectOption("#fps-select", "60");

  await expect
    .poll(() =>
      page.evaluate(() =>
        window.__phonecamCalls.some(
          ({ command, args }) =>
            command === "configure_stream" &&
            args.width === 1920 &&
            args.height === 1080 &&
            args.fps === 60
        )
      )
    )
    .toBe(true);

  await page.reload();
  await expect(page.locator("#resolution-select")).toHaveValue("1920x1080");
  await expect(page.locator("#fps-select")).toHaveValue("60");
});

test("shows a QR fallback URI", async ({ page }) => {
  await page.goto("/");
  await page.locator("#show-qr-btn").click();

  await expect(page.locator("#qr-code-panel")).toBeVisible();
  await expect(page.locator("#qr-code-image svg")).toHaveAttribute(
    "aria-label",
    "PhoneCam QR"
  );
  await expect(page.locator("#qr-code-uri")).toContainText("phonecam://192.0.2.10:7878");
});
