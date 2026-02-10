const { invoke } = window.__TAURI__.core;

let connected = false;

async function updateStatus() {
  try {
    const status = await invoke("get_status");
    connected = status.connected;
    const statusEl = document.getElementById("status-indicator");
    const dotEl = statusEl.querySelector(".status-dot");
    const textEl = statusEl.querySelector(".status-text");
    const connectBtn = document.getElementById("connect-btn");
    const disconnectBtn = document.getElementById("disconnect-btn");

    if (connected) {
      statusEl.classList.add("connected");
      statusEl.classList.remove("disconnected");
      dotEl.style.backgroundColor = "var(--secondary-color)";
      textEl.textContent = "Connected";
      connectBtn.disabled = true;
      disconnectBtn.disabled = false;
    } else {
      statusEl.classList.remove("connected");
      statusEl.classList.add("disconnected");
      dotEl.style.backgroundColor = "var(--error-color)";
      textEl.textContent = "Disconnected";
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
      li.textContent = `${device.name || 'Unknown'} (${device.ip}:${device.port})`;
      li.onclick = () => {
        document.getElementById("ip-input").value = device.ip;
        document.getElementById("port-input").value = device.port;
      };
      listEl.appendChild(li);
    });
  } catch (e) {
    console.error("Failed to get devices:", e);
  }
}

async function connect() {
  const ip = document.getElementById("ip-input").value;
  const port = parseInt(document.getElementById("port-input").value);
  if (!ip || !port) {
    alert("Please enter IP and Port");
    return;
  }
  
  try {
    await invoke("connect", { ip, port });
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
  const connectBtn = document.getElementById("connect-btn");
  const disconnectBtn = document.getElementById("disconnect-btn");
  
  if (connectBtn) connectBtn.addEventListener("click", connect);
  if (disconnectBtn) disconnectBtn.addEventListener("click", disconnect);
  
  setInterval(updateStatus, 1000);
  setInterval(updateDevices, 3000);
  
  updateStatus();
  updateDevices();
});
