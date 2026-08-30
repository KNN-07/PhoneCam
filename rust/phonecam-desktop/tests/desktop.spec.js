import { expect, test } from "@playwright/test";

const h264720 = { codec: "h264", width: 1280, height: 720, fps: 30 };
const h2644k60 = { codec: "h264", width: 3840, height: 2160, fps: 60 };
const hevc4k60 = { codec: "hevc", width: 3840, height: 2160, fps: 60 };

function connectedStatus(profiles = [h264720, h2644k60, hevc4k60]) {
  return {
    connected: true,
    state: "connected",
    last_error: null,
    supported_profiles: profiles,
    active_profile: h264720,
    output_format: { width: 1280, height: 720, fps: 30, pixel_format: "nv12" }
  };
}

test.beforeEach(async ({ context }) => {
  await context.addInitScript(() => {
    window.__phonecamCalls = [];
    window.__phonecamRejectConfiguration = false;
    window.__phonecamStatus = {
      connected: false,
      state: "listening",
      last_error: null,
      supported_profiles: [],
      active_profile: null,
      output_format: null
    };
    window.__TAURI__ = {
      core: {
        invoke: async (command, args = {}) => {
          window.__phonecamCalls.push({ command, args });
          switch (command) {
            case "get_status":
              return window.__phonecamStatus;
            case "configure_stream": {
              if (window.__phonecamRejectConfiguration) throw new Error("Unsupported");
              const requestedCodec = args.codec === "auto" ? "h264" : args.codec;
              const exact = window.__phonecamStatus.supported_profiles.find(
                (profile) =>
                  profile.codec === requestedCodec &&
                  profile.width === args.width &&
                  profile.height === args.height &&
                  profile.fps === args.fps
              );
              const fallback = window.__phonecamStatus.supported_profiles.find(
                (profile) =>
                  profile.codec === "h264" &&
                  profile.width === args.width &&
                  profile.height === args.height &&
                  profile.fps === args.fps
              );
              const applied = exact || fallback;
              if (!applied) throw new Error("Unsupported");
              window.__phonecamStatus.active_profile = applied;
              return applied;
            }
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

test("renders five resolutions, three rates, and three codec preferences", async ({ page }) => {
  await page.goto("/");
  await expect(page).toHaveTitle("PhoneCam");
  await expect(page.locator("#resolution-select option")).toHaveCount(5);
  await expect(page.locator("#fps-select option")).toHaveCount(3);
  await expect(page.locator("#codec-select option")).toHaveCount(3);
  await expect(page.locator("#status-indicator .status-text")).toHaveText("listening");
  await expect(page.locator("#switch-camera-btn")).toBeDisabled();
});

test("invokes and persists an advertised 4K60 HEVC profile", async ({ page }) => {
  await page.goto("/");
  await page.evaluate((status) => { window.__phonecamStatus = status; }, connectedStatus());
  await expect(page.locator("#switch-camera-btn")).toBeEnabled({ timeout: 2_000 });

  await page.selectOption("#resolution-select", "3840x2160");
  await page.selectOption("#fps-select", "60");
  await page.selectOption("#codec-select", "hevc");

  await expect.poll(() => page.evaluate(() => window.__phonecamCalls.filter(
    ({ command, args }) => command === "configure_stream" &&
      args.width === 3840 && args.height === 2160 && args.fps === 60 && args.codec === "hevc"
  ).length)).toBe(1);
  await expect(page.locator("#active-profile")).toContainText("HEVC · 3840×2160 · 60 FPS");

  await page.reload();
  await expect(page.locator("#resolution-select")).toHaveValue("3840x2160");
  await expect(page.locator("#fps-select")).toHaveValue("60");
  await expect(page.locator("#codec-select")).toHaveValue("hevc");
});

test("resolves Auto to H.264 when only H.264 is advertised", async ({ page }) => {
  await page.goto("/");
  await page.evaluate((status) => { window.__phonecamStatus = status; }, connectedStatus([h264720, h2644k60]));
  await expect(page.locator("#switch-camera-btn")).toBeEnabled({ timeout: 2_000 });
  await page.selectOption("#resolution-select", "3840x2160");
  await page.selectOption("#fps-select", "60");
  await page.selectOption("#codec-select", "auto");

  await expect.poll(() => page.evaluate(() => window.__phonecamCalls.some(
    ({ command, args }) => command === "configure_stream" && args.codec === "auto" && args.fps === 60
  ))).toBe(true);
  await expect(page.locator("#active-profile")).toContainText("H.264 · 3840×2160 · 60 FPS");
});

test("disables unsupported tuples and rolls back rejected changes", async ({ page }) => {
  await page.goto("/");
  await page.evaluate((status) => {
    window.__phonecamStatus = status;
    window.alert = () => {};
  }, connectedStatus([h264720, { codec: "h264", width: 1920, height: 1080, fps: 30 }]));
  await expect(page.locator("#switch-camera-btn")).toBeEnabled({ timeout: 2_000 });
  await expect(page.locator('#resolution-select option[value="3840x2160"]')).toBeDisabled();

  await page.evaluate(() => { window.__phonecamRejectConfiguration = true; });
  await page.selectOption("#resolution-select", "1920x1080");
  await expect(page.locator("#resolution-select")).toHaveValue("1280x720");
  await expect(page.locator("#active-profile")).toContainText("H.264 · 1280×720 · 30 FPS");
  await page.reload();
  await expect(page.locator("#resolution-select")).toHaveValue("1280x720");
});

test("shows a QR fallback URI", async ({ page }) => {
  await page.goto("/");
  await page.locator("#show-qr-btn").click();
  await expect(page.locator("#qr-code-panel")).toBeVisible();
  await expect(page.locator("#qr-code-image svg")).toHaveAttribute("aria-label", "PhoneCam QR");
  await expect(page.locator("#qr-code-uri")).toContainText("phonecam://192.0.2.10:7878");
});
