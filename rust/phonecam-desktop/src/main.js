const { invoke } = window.__TAURI__.core;

let currentStatus = { connected: false, state: "Disconnected" };

const DEFAULT_SETTINGS = {
  resolution: "1280x720",
  fps: "30"
};

function loadSettings() {
  try {
    const stored = localStorage.getItem("phonecam_settings");
    const settings = stored ? JSON.parse(stored) : DEFAULT_SETTINGS;
    
    const resSelect = document.getElementById("resolution-select");
    const fpsSelect = document.getElementById("fps-select");
    
    if (resSelect) resSelect.value = settings.resolution;
    if (fpsSelect) fpsSelect.value = settings.fps;
    
    console.log("Settings loaded:", settings);
  } catch (e) {
    console.error("Failed to load settings:", e);
  }
}

function saveSettings() {
  try {
    const settings = {
      resolution: document.getElementById("resolution-select").value,
      fps: document.getElementById("fps-select").value
    };
    localStorage.setItem("phonecam_settings", JSON.stringify(settings));
    console.log("Settings saved:", settings);
  } catch (e) {
    console.error("Failed to save settings:", e);
  }
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

    if (status.connected) {
      statusEl.classList.add("connected");
      statusEl.classList.remove("disconnected");
      dotEl.style.backgroundColor = "var(--secondary-color)";
      
      const settingsFps = document.getElementById("fps-select").value;
      fpsEl.textContent = `${settingsFps} FPS (Target)`; 
      fpsEl.style.display = "inline";

      connectBtn.disabled = true;
      disconnectBtn.disabled = false;
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
    }
  } catch (e) {
    console.error("Failed to get status:", e);
  }
}

async function updateDevices() {
  try {
    const devices = await invoke("get_discovered_devices");
    const listEl = document.getElementById("device-list");
    
    if (!devices || devices.length === 0) {
      listEl.innerHTML = '<li class="device-item empty-state">Scanning for devices...</li>';
      return;
    }

    listEl.innerHTML = "";
    devices.forEach(device => {
      const li = document.createElement("li");
      li.className = "device-item";
      
      const infoDiv = document.createElement("div");
      infoDiv.textContent = `${device.name || 'Unknown'} (${device.ip}:${device.port})`;
      
      const connectBtn = document.createElement("button");
      connectBtn.className = "btn primary-btn";
      connectBtn.textContent = "Connect";
      connectBtn.style.padding = "5px 10px";
      connectBtn.style.fontSize = "0.8rem";
      connectBtn.onclick = (e) => {
        e.stopPropagation();
        connect(device.ip, device.port);
      };
      
      li.onclick = () => {
        document.getElementById("ip-input").value = device.ip;
        document.getElementById("port-input").value = device.port;
      };
      
      li.appendChild(infoDiv);
      li.appendChild(connectBtn);
      listEl.appendChild(li);
    });
  } catch (e) {
    console.error("Failed to get devices:", e);
  }
}

async function connect(ipArg, portArg) {
  let ip = ipArg;
  let port = portArg;
  
  if (!ip || !port) {
    ip = document.getElementById("ip-input").value;
    port = parseInt(document.getElementById("port-input").value);
  }
  
  if (!ip || !port) {
    alert("Please enter IP and Port");
    return;
  }
  
  try {
    saveSettings(); 
    
    await invoke("connect", { ip, port: parseInt(port) });
    updateStatus();
  } catch (e) {
    alert("Connection failed: " + e);
  }
}

async function disconnect() {
  try {
    await invoke("disconnect");
    updateStatus();
  } catch (e) {
    alert("Disconnect failed: " + e);
  }
}

window.addEventListener("DOMContentLoaded", () => {
  loadSettings();

  const connectBtn = document.getElementById("connect-btn");
  const disconnectBtn = document.getElementById("disconnect-btn");
  const resolutionSelect = document.getElementById("resolution-select");
  const fpsSelect = document.getElementById("fps-select");
  
  if (connectBtn) connectBtn.addEventListener("click", () => connect());
  if (disconnectBtn) disconnectBtn.addEventListener("click", disconnect);
  
  if (resolutionSelect) resolutionSelect.addEventListener("change", saveSettings);
  if (fpsSelect) fpsSelect.addEventListener("change", saveSettings);
  
  setInterval(updateStatus, 1000);
  setInterval(updateDevices, 3000);
  
  updateStatus();
  updateDevices();
});
