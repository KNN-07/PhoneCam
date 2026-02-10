#![allow(dead_code)]

use std::{
    collections::HashSet,
    net::{IpAddr, SocketAddr},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use thiserror::Error;

pub const SERVICE_TYPE: &str = "_phonecam._tcp.local.";
const DEFAULT_MDNS_PORT: u16 = 5353;
const VERSION_PROPERTY_KEY: &str = "version";
const QR_SCHEME_PREFIX: &str = "phonecam://";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DiscoveredService {
    pub name: String,
    pub ip: IpAddr,
    pub port: u16,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QrCodeConnectionInfo {
    pub name: String,
    pub ip: IpAddr,
    pub port: u16,
}

#[derive(Debug, Error)]
pub enum DiscoveryError {
    #[error("mDNS error: {0}")]
    Mdns(#[from] mdns_sd::Error),

    #[error("mDNS browse channel closed")]
    BrowseChannelClosed,

    #[error("invalid QR code URI: {0}")]
    InvalidQrCodeUri(String),
}

#[derive(Clone)]
pub struct ServicePublisher {
    daemon: ServiceDaemon,
    fullname: String,
}

impl ServicePublisher {
    pub fn publish(device_name: &str, port: u16, version: &str) -> Result<Self, DiscoveryError> {
        Self::publish_with_mdns_port(device_name, port, version, DEFAULT_MDNS_PORT)
    }

    pub fn publish_with_mdns_port(
        device_name: &str,
        port: u16,
        version: &str,
        mdns_port: u16,
    ) -> Result<Self, DiscoveryError> {
        let daemon = ServiceDaemon::new_with_port(mdns_port)?;
        let host_name = build_host_name(device_name);
        let properties = [(VERSION_PROPERTY_KEY, version)];

        let service_info = ServiceInfo::new(
            SERVICE_TYPE,
            device_name,
            &host_name,
            "",
            port,
            properties.as_slice(),
        )?
        .enable_addr_auto();

        let fullname = service_info.get_fullname().to_string();
        daemon.register(service_info)?;

        Ok(Self { daemon, fullname })
    }

    pub fn fullname(&self) -> &str {
        &self.fullname
    }
}

impl Drop for ServicePublisher {
    fn drop(&mut self) {
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.shutdown();
    }
}

#[derive(Clone)]
pub struct ServiceBrowser {
    daemon: ServiceDaemon,
}

impl ServiceBrowser {
    pub fn new() -> Result<Self, DiscoveryError> {
        Self::new_with_mdns_port(DEFAULT_MDNS_PORT)
    }

    pub fn new_with_mdns_port(mdns_port: u16) -> Result<Self, DiscoveryError> {
        let daemon = ServiceDaemon::new_with_port(mdns_port)?;
        Ok(Self { daemon })
    }

    pub async fn discover(&self, timeout: Duration) -> Result<Vec<DiscoveredService>, DiscoveryError> {
        let receiver = self.daemon.browse(SERVICE_TYPE)?;
        let mut discovered = HashSet::new();
        let deadline = std::time::Instant::now() + timeout;

        loop {
            let now = std::time::Instant::now();
            let Some(remaining) = deadline.checked_duration_since(now) else {
                break;
            };

            if remaining.is_zero() {
                break;
            }

            match tokio::time::timeout(remaining, receiver.recv_async()).await {
                Ok(Ok(ServiceEvent::ServiceResolved(service))) => {
                    let service_name = instance_name_from_fullname(service.get_fullname());
                    let version = service
                        .get_property_val_str(VERSION_PROPERTY_KEY)
                        .unwrap_or_default()
                        .to_string();

                    for address in service.get_addresses() {
                        discovered.insert(DiscoveredService {
                            name: service_name.clone(),
                            ip: address.to_ip_addr(),
                            port: service.get_port(),
                            version: version.clone(),
                        });
                    }
                }
                Ok(Ok(_)) => {}
                Ok(Err(_)) => return Err(DiscoveryError::BrowseChannelClosed),
                Err(_) => break,
            }
        }

        let _ = self.daemon.stop_browse(SERVICE_TYPE);

        let mut all_services: Vec<_> = discovered.into_iter().collect();
        all_services.sort_unstable_by(|a, b| {
            a.name
                .cmp(&b.name)
                .then_with(|| a.ip.to_string().cmp(&b.ip.to_string()))
                .then_with(|| a.port.cmp(&b.port))
                .then_with(|| a.version.cmp(&b.version))
        });

        Ok(all_services)
    }
}

impl Drop for ServiceBrowser {
    fn drop(&mut self) {
        let _ = self.daemon.stop_browse(SERVICE_TYPE);
        let _ = self.daemon.shutdown();
    }
}

pub fn format_qr_code_uri(ip: IpAddr, port: u16, device_name: &str) -> String {
    let host = match ip {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => format!("[{v6}]"),
    };

    format!("{QR_SCHEME_PREFIX}{host}:{port}?name={device_name}")
}

pub fn parse_qr_code_uri(uri: &str) -> Result<QrCodeConnectionInfo, DiscoveryError> {
    let payload = uri
        .strip_prefix(QR_SCHEME_PREFIX)
        .ok_or_else(|| DiscoveryError::InvalidQrCodeUri("missing phonecam:// scheme".to_string()))?;

    let (authority, query) = payload.split_once('?').ok_or_else(|| {
        DiscoveryError::InvalidQrCodeUri("missing ?name=DEVICE_NAME query parameter".to_string())
    })?;

    let socket_addr: SocketAddr = authority.parse().map_err(|_| {
        DiscoveryError::InvalidQrCodeUri("expected host:port authority".to_string())
    })?;

    let name = parse_name_from_query(query).ok_or_else(|| {
        DiscoveryError::InvalidQrCodeUri("missing required name query parameter".to_string())
    })?;

    Ok(QrCodeConnectionInfo {
        name,
        ip: socket_addr.ip(),
        port: socket_addr.port(),
    })
}

fn parse_name_from_query(query: &str) -> Option<String> {
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix("name=").map(ToString::to_string))
}

fn instance_name_from_fullname(fullname: &str) -> String {
    fullname
        .strip_suffix(SERVICE_TYPE)
        .unwrap_or(fullname)
        .trim_end_matches('.')
        .to_string()
}

fn build_host_name(device_name: &str) -> String {
    let base_label = sanitize_dns_label(device_name);
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_micros())
        .unwrap_or_default();

    format!("{base_label}-{unique}.local.")
}

fn sanitize_dns_label(value: &str) -> String {
    let mut out = String::new();
    let mut last_was_dash = false;

    for ch in value.chars() {
        let normalized = if ch.is_ascii_alphanumeric() {
            last_was_dash = false;
            ch.to_ascii_lowercase()
        } else if !last_was_dash {
            last_was_dash = true;
            '-'
        } else {
            continue;
        };

        out.push(normalized);
        if out.len() >= 48 {
            break;
        }
    }

    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "phonecam".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        net::{IpAddr, Ipv4Addr, Ipv6Addr, UdpSocket},
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use crate::{
        format_qr_code_uri, parse_qr_code_uri, QrCodeConnectionInfo, ServiceBrowser,
        ServicePublisher,
    };

    fn pick_unused_udp_port() -> u16 {
        let socket = UdpSocket::bind("127.0.0.1:0").expect("must bind an ephemeral udp port");
        socket
            .local_addr()
            .expect("socket should have local address")
            .port()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn publish_and_discover() {
        let mdns_port = pick_unused_udp_port();
        let service_port = 49_999;
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_micros();
        let device_name = format!("PhoneCam-Test-{}-{unique}", std::process::id());
        let version = "0.1.0-test";

        let browser = ServiceBrowser::new_with_mdns_port(mdns_port)
            .expect("browser should initialize on custom mDNS port");
        let _publisher = ServicePublisher::publish_with_mdns_port(
            &device_name,
            service_port,
            version,
            mdns_port,
        )
        .expect("publisher should register service");

        let discovered = browser
            .discover(Duration::from_secs(3))
            .await
            .expect("discovery should finish without errors");

        let matching: Vec<_> = discovered
            .iter()
            .filter(|service| {
                service.name == device_name
                    && service.port == service_port
                    && service.version == version
            })
            .collect();

        assert!(
            !matching.is_empty(),
            "expected to discover published service; discovered: {discovered:?}"
        );

        assert!(
            matching
                .iter()
                .any(|service| matches!(service.ip, IpAddr::V4(_) | IpAddr::V6(_))),
            "expected at least one resolved IPv4 or IPv6 address"
        );
    }

    #[test]
    fn qr_code_uri_format() {
        let ipv4_info = QrCodeConnectionInfo {
            name: "Pixel7".to_string(),
            ip: IpAddr::V4(Ipv4Addr::new(192, 168, 0, 42)),
            port: 5_555,
        };

        let ipv4_uri = format_qr_code_uri(ipv4_info.ip, ipv4_info.port, &ipv4_info.name);
        assert_eq!(ipv4_uri, "phonecam://192.168.0.42:5555?name=Pixel7");

        let parsed_ipv4 = parse_qr_code_uri(&ipv4_uri).expect("must parse ipv4 qr uri");
        assert_eq!(parsed_ipv4, ipv4_info);

        let ipv6_info = QrCodeConnectionInfo {
            name: "iPhone15".to_string(),
            ip: IpAddr::V6(Ipv6Addr::LOCALHOST),
            port: 6_666,
        };

        let ipv6_uri = format_qr_code_uri(ipv6_info.ip, ipv6_info.port, &ipv6_info.name);
        assert_eq!(ipv6_uri, "phonecam://[::1]:6666?name=iPhone15");

        let parsed_ipv6 = parse_qr_code_uri(&ipv6_uri).expect("must parse ipv6 qr uri");
        assert_eq!(parsed_ipv6, ipv6_info);
    }
}
