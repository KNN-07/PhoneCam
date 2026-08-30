const { invoke } = window.__TAURI__.core;

let currentStatus = { connected: false, state: "Disconnected" };
let currentCameraFront = false;
let lastAppliedStreamSettings = null;

const DEFAULT_SETTINGS = {
  resolution: "1280x720",
  fps: "30",
  codec: "h264"
};

function loadSettings() {
  try {
    const stored = localStorage.getItem("phonecam_settings");
    const settings = stored ? JSON.parse(stored) : DEFAULT_SETTINGS;
    
    const resSelect = document.getElementById("resolution-select");
    const fpsSelect = document.getElementById("fps-select");
    const codecSelect = document.getElementById("codec-select");

    if (resSelect) resSelect.value = settings.resolution || DEFAULT_SETTINGS.resolution;
    if (fpsSelect) fpsSelect.value = settings.fps || DEFAULT_SETTINGS.fps;
    if (codecSelect) codecSelect.value = settings.codec || DEFAULT_SETTINGS.codec;
    
    console.log("Settings loaded:", settings);
  } catch (e) {
    console.error("Failed to load settings:", e);
  }
}

function saveSettings() {
  try {
    const settings = {
      resolution: document.getElementById("resolution-select").value,
      fps: document.getElementById("fps-select").value,
      codec: document.getElementById("codec-select").value
    };
    localStorage.setItem("phonecam_settings", JSON.stringify(settings));
    console.log("Settings saved:", settings);
  } catch (e) {
    console.error("Failed to save settings:", e);
  }
}

function readStreamSettings() {
  const [width, height] = document
    .getElementById("resolution-select")
    .value
    .split("x")
    .map(Number);
  const fps = Number(document.getElementById("fps-select").value);
  const codec = document.getElementById("codec-select").value;
  return { width, height, fps, codec };
}

function profileLabel(profile) {
  if (!profile) return "Active: waiting for phone";
  const codec = profile.codec === "hevc" ? "HEVC" : "H.264";
  return `Active: ${codec} · ${profile.width}×${profile.height} · ${profile.fps} FPS`;
}

function showActiveProfile(profile) {
  const summary = document.getElementById("active-profile");
  if (summary) summary.textContent = profileLabel(profile);
}

function restoreControlsFromActiveProfile() {
  const profile = currentStatus.active_profile;
  if (!profile) return;
  document.getElementById("resolution-select").value = `${profile.width}x${profile.height}`;
  document.getElementById("fps-select").value = String(profile.fps);
  document.getElementById("codec-select").value = profile.codec;
  saveSettings();
}

async function applyStreamSettings() {
  if (!currentStatus.connected) return;

  const settings = readStreamSettings();
  const settingsKey = `${settings.codec}:${settings.width}x${settings.height}@${settings.fps}`;
  const profiles = currentStatus.supported_profiles || [];
  const tupleAdvertised = profiles.some((profile) =>
    profile.width === settings.width &&
    profile.height === settings.height &&
    profile.fps === settings.fps &&
    (settings.codec === "auto" || profile.codec === settings.codec)
  );
  if (profiles.length > 0 && !tupleAdvertised) return;
  if (settingsKey === lastAppliedStreamSettings) return;

  const applied = await invoke("configure_stream", settings);
  lastAppliedStreamSettings = settingsKey;
  currentStatus.active_profile = applied;
  showActiveProfile(applied);
}

async function handleSettingsChange() {
  saveSettings();
  updateStreamControlAvailability();
  try {
    await applyStreamSettings();
  } catch (e) {
    restoreControlsFromActiveProfile();
    alert("Unable to update stream settings: " + e);
  }
}

function updateCameraControlUi() {
  const cameraStateEl = document.getElementById("camera-state-text");
  const switchCameraBtn = document.getElementById("switch-camera-btn");

  if (cameraStateEl) {
    cameraStateEl.textContent = currentCameraFront ? "Front" : "Back";
  }

  if (switchCameraBtn) {
    switchCameraBtn.textContent = currentCameraFront
      ? "Switch to Back Camera"
      : "Switch to Front Camera";
    switchCameraBtn.disabled = !currentStatus.connected;
  }
}

function updateStreamControlAvailability() {
  const profiles = currentStatus.supported_profiles || [];
  if (!currentStatus.connected || profiles.length === 0) {
    document.querySelectorAll("#resolution-select option, #fps-select option, #codec-select option")
      .forEach((option) => { option.disabled = false; });
    return;
  }
  const settings = readStreamSettings();
  const supportsCodec = (profile, preference) =>
    preference === "auto" || profile.codec === preference;

  document.querySelectorAll("#resolution-select option").forEach((option) => {
    const [width, height] = option.value.split("x").map(Number);
    option.disabled = !profiles.some((profile) =>
      profile.width === width && profile.height === height
    );
  });
  document.querySelectorAll("#fps-select option").forEach((option) => {
    const fps = Number(option.value);
    option.disabled = !profiles.some((profile) =>
      profile.width === settings.width &&
      profile.height === settings.height &&
      profile.fps === fps
    );
  });
  document.querySelectorAll("#codec-select option").forEach((option) => {
    option.disabled = !profiles.some((profile) =>
      profile.width === settings.width &&
      profile.height === settings.height &&
      profile.fps === settings.fps &&
      supportsCodec(profile, option.value)
    );
  });
}

async function updateStatus() {
  try {
    const status = await invoke("get_status");
    currentStatus = status;
    
    const statusEl = document.getElementById("status-indicator");
    const dotEl = statusEl.querySelector(".status-dot");
    const textEl = statusEl.querySelector(".status-text");
    const fpsEl = statusEl.querySelector(".fps-counter");
    const connectBtn = document.getElementById("connect-btn");
    const disconnectBtn = document.getElementById("disconnect-btn");

    let displayState = status.state || (status.connected ? "Connected" : "Disconnected");
    
    textEl.textContent = displayState;
    showActiveProfile(status.active_profile);
    updateStreamControlAvailability();

    if (status.connected) {
      statusEl.classList.add("connected");
      statusEl.classList.remove("disconnected");
      dotEl.style.backgroundColor = "var(--secondary-color)";
      
      const activeFps = status.active_profile?.fps;
      fpsEl.textContent = activeFps ? `${activeFps} FPS` : "Negotiating";
      fpsEl.style.display = "inline";

      connectBtn.disabled = true;
      disconnectBtn.disabled = false;
      await applyStreamSettings();
    } else {
      statusEl.classList.remove("connected");
      statusEl.classList.add("disconnected");
      
      if (displayState === "Listening") {
          dotEl.style.backgroundColor = "#ffa500";
      } else {
          dotEl.style.backgroundColor = "var(--error-color)";
      }
      
      fpsEl.style.display = "none";
      connectBtn.disabled = false;
      disconnectBtn.disabled = true;
      lastAppliedStreamSettings = null;
    }

    updateCameraControlUi();
  } catch (e) {
    console.error("Failed to get status:", e);
  }
}

async function connect(ip, portArg) {
  const port = portArg || parseInt(document.getElementById("port-input").value);
  
  if (!ip || !port) {
    alert("Please enter IP and Port");
    return;
  }
  
  try {
    saveSettings(); 
    
    await invoke("connect", { ip, port: parseInt(port) });
    currentCameraFront = false;
    updateCameraControlUi();
    updateStatus();
  } catch (e) {
    alert("Connection failed: " + e);
  }
}

function startWifiReceiver() {
  return connect("wifi");
}

function startUsbReceiver() {
  const serial = document.getElementById("ip-input").value.trim();
  return connect(serial ? `usb:${serial}` : "usb");
}

async function disconnect() {
  try {
    await invoke("disconnect");
    currentCameraFront = false;
    updateCameraControlUi();
    updateStatus();
  } catch (e) {
    alert("Disconnect failed: " + e);
  }
}

async function switchCamera() {
  if (!currentStatus.connected) {
    alert("Connect to a phone before switching camera");
    return;
  }

  const nextFront = !currentCameraFront;
  const switchCameraBtn = document.getElementById("switch-camera-btn");
  if (switchCameraBtn) {
    switchCameraBtn.disabled = true;
  }

  try {
    await invoke("switch_camera", { front: nextFront });
    currentCameraFront = nextFront;
    updateCameraControlUi();
  } catch (e) {
    alert("Camera switch failed: " + e);
    updateCameraControlUi();
  }
}

async function showQrCode() {
  const panelEl = document.getElementById("qr-code-panel");
  const showBtn = document.getElementById("show-qr-btn");
  const hideBtn = document.getElementById("hide-qr-btn");
  const qrImageEl = document.getElementById("qr-code-image");
  const qrUriEl = document.getElementById("qr-code-uri");
  const qrUriListEl = document.getElementById("qr-code-uri-list");

  try {
    const [qrSvg, uris] = await Promise.all([
      invoke("generate_qr_code"),
      invoke("get_qr_connection_uris")
    ]);

    qrImageEl.innerHTML = qrSvg;
    qrUriEl.textContent = uris[0] || "No QR URI available";

    qrUriListEl.innerHTML = "";
    uris.forEach((uri, index) => {
      const li = document.createElement("li");
      li.className = "device-item";
      li.style.cursor = "text";
      li.textContent = index === 0 ? `${uri} (primary interface)` : uri;
      qrUriListEl.appendChild(li);
    });

    panelEl.hidden = false;
    showBtn.hidden = true;
    hideBtn.hidden = false;
  } catch (e) {
    alert("QR code generation failed: " + e);
  }
}

function hideQrCode() {
  const panelEl = document.getElementById("qr-code-panel");
  const showBtn = document.getElementById("show-qr-btn");
  const hideBtn = document.getElementById("hide-qr-btn");
  panelEl.hidden = true;
  showBtn.hidden = false;
  hideBtn.hidden = true;
}

window.addEventListener("DOMContentLoaded", () => {
  loadSettings();

  const connectBtn = document.getElementById("connect-btn");
  const usbConnectBtn = document.getElementById("usb-connect-btn");
  const disconnectBtn = document.getElementById("disconnect-btn");
  const showQrBtn = document.getElementById("show-qr-btn");
  const hideQrBtn = document.getElementById("hide-qr-btn");
  const switchCameraBtn = document.getElementById("switch-camera-btn");
  const resolutionSelect = document.getElementById("resolution-select");
  const fpsSelect = document.getElementById("fps-select");
  const codecSelect = document.getElementById("codec-select");
  
  if (connectBtn) connectBtn.addEventListener("click", startWifiReceiver);
  if (usbConnectBtn) usbConnectBtn.addEventListener("click", startUsbReceiver);
  if (disconnectBtn) disconnectBtn.addEventListener("click", disconnect);
  if (showQrBtn) showQrBtn.addEventListener("click", showQrCode);
  if (hideQrBtn) hideQrBtn.addEventListener("click", hideQrCode);
  if (switchCameraBtn) switchCameraBtn.addEventListener("click", switchCamera);
  
  if (resolutionSelect) resolutionSelect.addEventListener("change", handleSettingsChange);
  if (fpsSelect) fpsSelect.addEventListener("change", handleSettingsChange);
  
  if (codecSelect) codecSelect.addEventListener("change", handleSettingsChange);
  setInterval(updateStatus, 1000);
  
  updateCameraControlUi();
  updateStatus();
});
