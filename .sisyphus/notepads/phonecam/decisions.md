## 2026-02-10

- Chose `bincode` (serde mode) as the binary serializer due to straightforward `encode_to_vec` / `decode_from_slice` APIs and explicit Context7-backed guidance.
- Implemented wire format as: `[u32 BE length][u8 message_type][payload]`, where payload is serialized per concrete message type (not full enum serialization), to keep type-byte control explicit and protocol-stable.
- Modeled `AudioFrame` as a deprecated message type (`#[deprecated(note = "Reserved for v2")]`) and allowed codec round-trip tests without adding any runtime audio pipeline behavior.

- Implemented `phonecam-transport` as a single-connection TCP design (`PhoneCamServer::accept` handles one stream instance) to match v1 scope and simplify state management.
- Chose `watch::channel<ConnectionState>` for state visibility and testability, with transitions driven by connect/accept and handshake reception.
- Kept keepalive in transport layer using periodic framed `StatusUpdate` pings and 5-second receive timeout, avoiding protocol surface expansion.

- Implemented discovery API with explicit `ServicePublisher` (desktop-side registration) and `ServiceBrowser` (mobile-side browse) wrappers over `mdns-sd` to keep platform integration straightforward.
- Added `publish_with_mdns_port` / `new_with_mdns_port` constructors so tests can run against isolated mDNS ports while production defaults remain port 5353.
- Modeled discovered results as one record per resolved IP address (`{name, ip, port, version}`), preserving both IPv4 and IPv6 addresses returned by mDNS.
- Implemented QR data format as a simple URI parser/formatter (`phonecam://IP:PORT?name=DEVICE_NAME`) with IPv6 bracket handling (`[::1]:PORT`).

## 2026-02-10 (Wave 2)

- Tauri desktop app structure: Main crate at `rust/phonecam-desktop/src-tauri/` (following Tauri conventions), frontend assets at `rust/phonecam-desktop/` root.
- Desktop app uses background tokio task in `.setup()` to continuously run mDNS discovery every 3 seconds, updating shared state for frontend to query via `get_discovered_devices()` command.
- Connection management: Single client instance stored in `Arc<TokioMutex<Option<PhoneCamClient>>>` allows atomic connect/disconnect via Tauri commands while preventing multiple simultaneous connections.
