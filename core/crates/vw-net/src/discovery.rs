//! Discovery is an untrusted address hint. It never reads or grants app trust.
//! A fresh cryptographic ID is generated per process startup, independently of
//! the long-term DeviceId. Android binds DiscoveryCallback to NsdManager in T1.10.
use crate::{
    PROTOCOL_MAJOR,
    pairing::{PairingError, Result, entropy, validate_endpoints},
};
use std::{
    collections::BTreeMap,
    fmt,
    net::SocketAddr,
    time::{Duration, Instant},
};

pub const SERVICE_TYPE: &str = "_vworkbench._udp.local.";
pub const ANDROID_SERVICE_TYPE: &str = "_vworkbench._udp";
const MAX_DISCOVERIES: usize = 64;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DiscoveryId(String);
impl DiscoveryId {
    pub fn for_startup() -> Result<Self> {
        let bytes = entropy::<16>()?;
        let mut text = String::with_capacity(32);
        for byte in bytes {
            text.push(char::from(b"0123456789abcdef"[(byte >> 4) as usize]));
            text.push(char::from(b"0123456789abcdef"[(byte & 15) as usize]));
        }
        Ok(Self(text))
    }
    fn parse(text: &str) -> Result<Self> {
        if text.len() != 32 || !text.bytes().all(|byte| b"0123456789abcdef".contains(&byte)) {
            return Err(PairingError::Invalid("discovery id"));
        }
        Ok(Self(text.into()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for DiscoveryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DiscoveryId([REDACTED])")
    }
}

#[derive(Clone)]
pub struct Advertisement {
    id: DiscoveryId,
    endpoints: Vec<SocketAddr>,
}
impl Advertisement {
    pub fn for_startup(endpoints: Vec<SocketAddr>) -> Result<Self> {
        validate_endpoints(&endpoints)?;
        if endpoints
            .iter()
            .any(|endpoint| endpoint.port() != endpoints[0].port())
        {
            return Err(PairingError::Invalid("discovery port"));
        }
        Ok(Self {
            id: DiscoveryId::for_startup()?,
            endpoints,
        })
    }
    pub const fn id(&self) -> &DiscoveryId {
        &self.id
    }
    pub fn endpoints(&self) -> &[SocketAddr] {
        &self.endpoints
    }
    /// These are the only TXT properties. No names, DeviceId, addresses or pins.
    pub fn txt(&self) -> [(String, String); 2] {
        [
            ("id".into(), self.id.0.clone()),
            ("v".into(), PROTOCOL_MAJOR.to_string()),
        ]
    }
    pub fn instance(&self) -> String {
        format!("vw-{}", self.id.as_str())
    }
    pub fn hostname(&self) -> String {
        format!("vw-{}.local.", self.id.as_str())
    }
}
impl fmt::Debug for Advertisement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Advertisement([REDACTED])")
    }
}

#[derive(Clone)]
pub struct DiscoveryHint {
    id: DiscoveryId,
    endpoints: Vec<SocketAddr>,
}
impl DiscoveryHint {
    pub fn from_untrusted(
        service_type: &str,
        txt: &[(String, String)],
        endpoints: Vec<SocketAddr>,
    ) -> Result<Self> {
        if ![SERVICE_TYPE, ANDROID_SERVICE_TYPE].contains(&service_type) || txt.len() != 2 {
            return Err(PairingError::Invalid("discovery TXT"));
        }
        let id = txt
            .iter()
            .find(|(key, _)| key == "id")
            .map(|(_, value)| value)
            .ok_or(PairingError::Invalid("discovery TXT"))?;
        let version = txt
            .iter()
            .find(|(key, _)| key == "v")
            .map(|(_, value)| value)
            .ok_or(PairingError::Invalid("discovery TXT"))?;
        if version != &PROTOCOL_MAJOR.to_string() {
            return Err(PairingError::Invalid("discovery version"));
        }
        validate_endpoints(&endpoints)?;
        Ok(Self {
            id: DiscoveryId::parse(id)?,
            endpoints,
        })
    }
    pub const fn id(&self) -> &DiscoveryId {
        &self.id
    }
    pub fn endpoints(&self) -> &[SocketAddr] {
        &self.endpoints
    }
}
impl fmt::Debug for DiscoveryHint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DiscoveryHint([REDACTED])")
    }
}

/// Android's callback owns NsdManager registration/discovery and calls the
/// bounded catalog with resolved hints. Stop must unregister both listeners.
pub trait DiscoveryCallback: Send + Sync {
    fn start(&self, advertisement: &Advertisement) -> Result<()>;
    fn stop(&self) -> Result<()>;
}

#[derive(Default)]
pub struct DiscoveryCatalog {
    entries: BTreeMap<DiscoveryId, (DiscoveryHint, Instant)>,
}
impl DiscoveryCatalog {
    pub fn observe(&mut self, hint: DiscoveryHint, now: Instant) -> Result<()> {
        self.expire(now);
        if self.entries.len() >= MAX_DISCOVERIES && !self.entries.contains_key(hint.id()) {
            return Err(PairingError::Capacity);
        }
        let expires = now
            .checked_add(Duration::from_secs(60))
            .ok_or(PairingError::Capacity)?;
        self.entries.insert(hint.id.clone(), (hint, expires));
        Ok(())
    }
    pub fn expire(&mut self, now: Instant) {
        self.entries.retain(|_, (_, expires)| now < *expires);
    }
    pub fn remove(&mut self, id: &DiscoveryId) {
        self.entries.remove(id);
    }
    pub fn hints(&self) -> impl Iterator<Item = &DiscoveryHint> {
        self.entries.values().map(|(hint, _)| hint)
    }
}

#[cfg(feature = "mdns")]
pub mod desktop {
    use super::*;
    use mdns_sd::{IfKind, ServiceDaemon, ServiceEvent, ServiceInfo};
    use std::net::{IpAddr, SocketAddrV6};

    pub struct MdnsDiscovery {
        daemon: ServiceDaemon,
        fullname: Option<String>,
        closed: bool,
    }
    impl MdnsDiscovery {
        /// Advertise/browse only explicitly selected local interfaces. No admin
        /// changes or firewall commands are issued by this adapter.
        pub fn start(advertisement: &Advertisement) -> Result<Self> {
            let daemon = ServiceDaemon::new().map_err(|_| PairingError::Connection)?;
            let mut service = Self {
                daemon,
                fullname: None,
                closed: false,
            };
            service
                .daemon
                .disable_interface(IfKind::All)
                .map_err(|_| PairingError::Connection)?;
            let addresses: Vec<IpAddr> =
                advertisement.endpoints.iter().map(SocketAddr::ip).collect();
            for address in &addresses {
                service
                    .daemon
                    .enable_interface(IfKind::Addr(*address))
                    .map_err(|_| PairingError::Connection)?;
            }
            let txt = advertisement.txt();
            let mut info = ServiceInfo::new(
                SERVICE_TYPE,
                &advertisement.instance(),
                &advertisement.hostname(),
                addresses.as_slice(),
                advertisement.endpoints[0].port(),
                txt.as_slice(),
            )
            .map_err(|_| PairingError::Connection)?;
            info.set_interfaces(addresses.into_iter().map(IfKind::Addr).collect());
            service.fullname = Some(info.get_fullname().to_owned());
            service
                .daemon
                .register(info)
                .map_err(|_| PairingError::Connection)?;
            Ok(service)
        }
        /// Blocking desktop worker API, capped to 15 seconds and 256 events.
        /// Results remain untrusted and require QR/PAKE authentication.
        pub fn browse_for(&self, duration: Duration) -> Result<Vec<DiscoveryHint>> {
            if self.closed || duration.is_zero() || duration > Duration::from_secs(15) {
                return Err(PairingError::Invalid("discovery duration"));
            }
            let receiver = self
                .daemon
                .browse(SERVICE_TYPE)
                .map_err(|_| PairingError::Connection)?;
            let deadline = Instant::now() + duration;
            let mut catalog = DiscoveryCatalog::default();
            for _ in 0..256 {
                let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                    break;
                };
                let Ok(event) = receiver.recv_timeout(remaining) else {
                    break;
                };
                if let ServiceEvent::ServiceResolved(info) = event {
                    if info.get_properties().len() != 2 || info.get_addresses().len() > 8 {
                        continue;
                    }
                    let (Some(id), Some(version)) = (
                        info.get_property_val_str("id"),
                        info.get_property_val_str("v"),
                    ) else {
                        continue;
                    };
                    let addresses: Vec<SocketAddr> = info
                        .get_addresses()
                        .iter()
                        .filter_map(|address| match address {
                            mdns_sd::ScopedIp::V4(v4) => {
                                Some(SocketAddr::new(IpAddr::V4(*v4.addr()), info.get_port()))
                            }
                            mdns_sd::ScopedIp::V6(v6) => Some(SocketAddr::V6(SocketAddrV6::new(
                                *v6.addr(),
                                info.get_port(),
                                0,
                                v6.scope_id().index,
                            ))),
                            _ => None,
                        })
                        .collect();
                    if let Ok(hint) = DiscoveryHint::from_untrusted(
                        SERVICE_TYPE,
                        &[("id".into(), id.into()), ("v".into(), version.into())],
                        addresses,
                    ) {
                        let _ = catalog.observe(hint, Instant::now());
                    }
                }
            }
            self.daemon
                .stop_browse(SERVICE_TYPE)
                .map_err(|_| PairingError::Connection)?;
            Ok(catalog.hints().cloned().collect())
        }
        pub fn close(&mut self) -> Result<()> {
            if self.closed {
                return Ok(());
            }
            if let Some(fullname) = self.fullname.take() {
                let _ = self.daemon.unregister(&fullname);
            }
            let receiver = self
                .daemon
                .shutdown()
                .map_err(|_| PairingError::Connection)?;
            receiver
                .recv_timeout(Duration::from_secs(2))
                .map_err(|_| PairingError::Timeout)?;
            self.closed = true;
            Ok(())
        }
    }
    impl Drop for MdnsDiscovery {
        fn drop(&mut self) {
            let _ = self.close();
        }
    }
    impl fmt::Debug for MdnsDiscovery {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("MdnsDiscovery([REDACTED])")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discovery_rotates_has_exact_private_txt_and_no_device_input() -> Result<()> {
        let endpoint = SocketAddr::from(([127, 0, 0, 1], 47831));
        let a = Advertisement::for_startup(vec![endpoint])?;
        let b = Advertisement::for_startup(vec![endpoint])?;
        assert_ne!(a.id(), b.id());
        assert_eq!(a.txt().map(|(key, _)| key), ["id", "v"]);
        assert!(!format!("{a:?}").contains("127.0.0.1"));
        let hint = DiscoveryHint::from_untrusted(SERVICE_TYPE, &a.txt(), vec![endpoint])?;
        let mut catalog = DiscoveryCatalog::default();
        let now = Instant::now();
        catalog.observe(hint, now)?;
        assert_eq!(catalog.hints().count(), 1);
        catalog.expire(now + Duration::from_secs(60));
        assert_eq!(catalog.hints().count(), 0);
        let mut txt = a.txt().to_vec();
        txt.push(("device_id".into(), "forbidden".into()));
        assert!(DiscoveryHint::from_untrusted(SERVICE_TYPE, &txt, vec![endpoint]).is_err());
        let mut wrong = a.txt();
        wrong[1].1 = "999".into();
        assert!(DiscoveryHint::from_untrusted(SERVICE_TYPE, &wrong, vec![endpoint]).is_err());
        Ok(())
    }
}
