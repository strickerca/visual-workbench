use super::{SessionError, SessionResult, SessionService};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Instant;
use vw_net::discovery::{Advertisement, DiscoveryCatalog, DiscoveryHint};

#[derive(Clone, Debug, uniffi::Record)]
pub struct DiscoveryProperty {
    pub key: String,
    pub value: String,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct DiscoveryAdvertisement {
    pub instance: String,
    pub service_type: String,
    pub port: u16,
    pub properties: Vec<DiscoveryProperty>,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct DiscoveredPeer {
    pub discovery_id: String,
    pub endpoints: Vec<String>,
}
static DISCOVERIES: AtomicUsize = AtomicUsize::new(0);
struct Permit;
impl Permit {
    fn acquire() -> SessionResult<Self> {
        DISCOVERIES
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < 4).then_some(n + 1)
            })
            .map_err(|_| SessionError::Backpressure)?;
        Ok(Self)
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        DISCOVERIES.fetch_sub(1, Ordering::AcqRel);
    }
}
struct State {
    catalog: DiscoveryCatalog,
    own_id: String,
    #[cfg(windows)]
    desktop: Option<vw_net::discovery::desktop::MdnsDiscovery>,
    permit: Option<Permit>,
}
#[derive(uniffi::Object)]
pub struct SessionDiscovery {
    advertisement: Option<DiscoveryAdvertisement>,
    worker: crate::worker::Worker<State>,
}
fn addresses(values: Vec<String>) -> SessionResult<Vec<std::net::SocketAddr>> {
    if values.is_empty() || values.len() > 8 {
        return Err(SessionError::Invalid);
    }
    values
        .into_iter()
        .map(|value| {
            let validated = super::validate_session_endpoint(value, false)?;
            validated.parse().map_err(|_| SessionError::Invalid)
        })
        .collect()
}
fn observe(state: &mut State, hint: DiscoveryHint) -> SessionResult<()> {
    for endpoint in hint.endpoints() {
        super::validate_session_endpoint(endpoint.to_string(), false)?;
    }
    if hint.id().as_str() != state.own_id {
        state.catalog.observe(hint, Instant::now())?;
    }
    Ok(())
}
fn snapshot(state: &mut State) -> Vec<DiscoveredPeer> {
    state.catalog.expire(Instant::now());
    state
        .catalog
        .hints()
        .map(|hint| DiscoveredPeer {
            discovery_id: hint.id().as_str().into(),
            endpoints: hint.endpoints().iter().map(ToString::to_string).collect(),
        })
        .collect()
}
#[uniffi::export]
impl SessionService {
    /// Windows owns a bounded native mDNS worker. Android owns platform
    /// NsdManager registration and feeds its resolved, still-untrusted hints.
    pub async fn create_discovery(
        &self,
        endpoints: Vec<String>,
    ) -> SessionResult<Arc<SessionDiscovery>> {
        let endpoints = addresses(endpoints)?;
        let permit = Permit::acquire()?;
        crate::worker::startup(move || {
            Ok((|| -> SessionResult<_> {
                let offer = Advertisement::for_startup(endpoints)?;
                let advertisement = DiscoveryAdvertisement {
                    instance: offer.instance(),
                    service_type: vw_net::discovery::ANDROID_SERVICE_TYPE.into(),
                    port: offer.endpoints()[0].port(),
                    properties: offer
                        .txt()
                        .into_iter()
                        .map(|(key, value)| DiscoveryProperty { key, value })
                        .collect(),
                };
                #[cfg(windows)]
                let desktop = Some(vw_net::discovery::desktop::MdnsDiscovery::start(&offer)?);
                let state = State {
                    catalog: DiscoveryCatalog::default(),
                    own_id: offer.id().as_str().into(),
                    #[cfg(windows)]
                    desktop,
                    permit: Some(permit),
                };
                Ok(Arc::new(SessionDiscovery {
                    advertisement: Some(advertisement),
                    worker: crate::worker::Worker::new("vw-discovery", state, 2)?,
                }))
            })())
        })
        .await?
    }
    pub async fn browse_discovery(
        &self,
        local_addresses: Vec<String>,
    ) -> SessionResult<Arc<SessionDiscovery>> {
        if local_addresses.len() > 8 {
            return Err(SessionError::Invalid);
        }
        let selected = local_addresses
            .into_iter()
            .map(|address| {
                if address.len() > 100 {
                    return Err(SessionError::Invalid);
                }
                let text = if address.contains(':') {
                    format!("[{address}]:0")
                } else {
                    format!("{address}:0")
                };
                let canonical = super::validate_session_endpoint(text, true)?;
                canonical
                    .parse::<std::net::SocketAddr>()
                    .map_err(|_| SessionError::Invalid)
            })
            .collect::<SessionResult<Vec<_>>>()?;
        let permit = Permit::acquire()?;
        crate::worker::startup(move || {
            Ok((|| -> SessionResult<_> {
                #[cfg(windows)]
                let desktop = Some(vw_net::discovery::desktop::MdnsDiscovery::browse_on(
                    &selected,
                )?);
                #[cfg(not(windows))]
                let _ = selected;
                let state = State {
                    catalog: DiscoveryCatalog::default(),
                    own_id: String::new(),
                    #[cfg(windows)]
                    desktop,
                    permit: Some(permit),
                };
                Ok(Arc::new(SessionDiscovery {
                    advertisement: None,
                    worker: crate::worker::Worker::new("vw-discovery", state, 2)?,
                }))
            })())
        })
        .await?
    }
}
#[uniffi::export]
impl SessionDiscovery {
    pub fn advertisement(&self) -> Option<DiscoveryAdvertisement> {
        self.advertisement.clone()
    }
    pub async fn observe(
        &self,
        service_type: String,
        properties: Vec<DiscoveryProperty>,
        endpoints: Vec<String>,
    ) -> SessionResult<()> {
        if service_type.len() > 64
            || properties.len() != 2
            || properties
                .iter()
                .any(|p| p.key.len() > 8 || p.value.len() > 32)
        {
            return Err(SessionError::Invalid);
        }
        let hint = DiscoveryHint::from_untrusted(
            &service_type,
            &properties
                .into_iter()
                .map(|p| (p.key, p.value))
                .collect::<Vec<_>>(),
            addresses(endpoints)?,
        )?;
        self.worker
            .call(move |state| Ok(observe(state, hint)))
            .await?
    }
    pub async fn forget(&self, discovery_id: String) -> SessionResult<()> {
        if discovery_id.len() != 32
            || !discovery_id
                .bytes()
                .all(|c| b"0123456789abcdef".contains(&c))
        {
            return Err(SessionError::Invalid);
        }
        self.worker
            .call(move |state| {
                let id = state
                    .catalog
                    .hints()
                    .find(|hint| hint.id().as_str() == discovery_id)
                    .map(|hint| hint.id().clone());
                if let Some(id) = id {
                    state.catalog.remove(&id);
                }
                Ok(())
            })
            .await
            .map_err(SessionError::from)
    }
    pub async fn snapshot(&self) -> SessionResult<Vec<DiscoveredPeer>> {
        self.worker
            .call(|state| {
                Ok((|| -> SessionResult<_> {
                    #[cfg(windows)]
                    if let Some(desktop) = &state.desktop {
                        let hints = desktop.browse_for(std::time::Duration::from_millis(500))?;
                        for hint in hints {
                            if let Err(error) = observe(state, hint)
                                && !matches!(
                                    error,
                                    SessionError::Backpressure | SessionError::Invalid
                                )
                            {
                                return Err(error);
                            }
                        }
                    }
                    Ok(snapshot(state))
                })())
            })
            .await?
    }
    pub async fn close(&self) -> SessionResult<()> {
        self.worker
            .shutdown(|state| {
                #[cfg(windows)]
                if let Some(mut desktop) = state.desktop.take() {
                    desktop.close().map_err(|_| crate::CoreError::Worker)?;
                }
                state.catalog = DiscoveryCatalog::default();
                state.permit.take();
                Ok(())
            })
            .await
            .map_err(SessionError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_bounds_untrusted_hints_and_never_reports_own_offer() -> SessionResult<()> {
        let offer = Advertisement::for_startup(vec![
            "127.0.0.1:41234"
                .parse()
                .map_err(|_| SessionError::Invalid)?,
        ])?;
        let mut state = State {
            catalog: DiscoveryCatalog::default(),
            own_id: offer.id().as_str().into(),
            #[cfg(windows)]
            desktop: None,
            permit: None,
        };
        observe(
            &mut state,
            DiscoveryHint::from_untrusted(
                vw_net::discovery::SERVICE_TYPE,
                &offer.txt(),
                offer.endpoints().to_vec(),
            )?,
        )?;
        assert!(snapshot(&mut state).is_empty());
        for n in 0..64 {
            let txt = [
                ("id".into(), format!("{n:032x}")),
                ("v".into(), vw_net::PROTOCOL_MAJOR.to_string()),
            ];
            observe(
                &mut state,
                DiscoveryHint::from_untrusted(
                    vw_net::discovery::SERVICE_TYPE,
                    &txt,
                    offer.endpoints().to_vec(),
                )?,
            )?;
        }
        let txt = [
            ("id".into(), format!("{:032x}", 65)),
            ("v".into(), vw_net::PROTOCOL_MAJOR.to_string()),
        ];
        assert!(matches!(
            observe(
                &mut state,
                DiscoveryHint::from_untrusted(
                    vw_net::discovery::SERVICE_TYPE,
                    &txt,
                    offer.endpoints().to_vec()
                )?
            ),
            Err(SessionError::Backpressure)
        ));
        assert_eq!(snapshot(&mut state).len(), 64);
        assert!(addresses(vec!["8.8.8.8:41234".into()]).is_err());
        Ok(())
    }
}
