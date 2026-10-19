//! Read-only Paint observations belong to the actual authenticated host, source
//! lease and joined pinned helper. Import bytes never supply those facts.
use super::*;

#[derive(uniffi::Object)]
pub struct RemoteCompatibilityObservation {
    #[cfg(windows)]
    owned: Mutex<Option<Owned>>,
}
#[cfg(windows)]
struct Owned {
    hub: std::sync::Weak<RemoteHub>,
    binding: Binding,
    target: Target,
    identity: vw_remote::profile::ImageIdentity,
    profile_digest: String,
    source: Arc<vw_remote_host::editor_source::SourceLease>,
    cancel: Arc<vw_remote_host::process::Cancellation>,
    helper: Arc<vw_remote_host::editor_probe_runner::PaintObservationHelper>,
    observation: vw_remote_host::editor_probe_runner::PaintControlsObservation,
    receipt: Vec<u8>,
}

#[cfg(windows)]
impl Owned {
    fn current(&self) -> SessionResult<()> {
        self.cancel.check().map_err(failure)?;
        let hub = self.hub.upgrade().ok_or(SessionError::Closed)?;
        let s = hub.state.lock().map_err(|_| SessionError::Worker)?;
        RemoteHub::current(&s)?;
        if s.host_retiring
            || s.binding.as_ref() != Some(&self.binding)
            || s.selected.as_ref().map(|v| &v.target) != Some(&self.target)
            || s.authority.as_ref().is_none_or(|a| {
                a.identity != self.identity || a.profile_digest != self.profile_digest
            })
            || s.runtime
                .as_ref()
                .and_then(|r| r.observation.as_ref())
                .is_none_or(|helper| !Arc::ptr_eq(helper, &self.helper))
        {
            return Err(SessionError::Closed);
        }
        Ok(())
    }
    fn verify_source(&self) -> SessionResult<()> {
        self.source
            .verify_observation(&self.target, std::process::id())
            .map_err(failure)
    }
}

// Every result is deliberately NotTested. A read-only controls witness says
// which tool/settings were observed; it proves no drawing or physical gesture.
#[cfg(any(windows, test))]
fn receipt(
    identity: &vw_remote::profile::ImageIdentity,
    profile: &str,
    tool: &str,
    settings: &str,
) -> SessionResult<Vec<u8>> {
    let mut image = serde_json::to_value(identity).map_err(|_| SessionError::Invalid)?;
    let object = image.as_object_mut().ok_or(SessionError::Invalid)?;
    object.insert("editor".into(), serde_json::json!("Paint"));
    object.insert("shortcutProfileDigest".into(), serde_json::json!(profile));
    let observations = [
        "Drawing",
        "Pressure",
        "Hover",
        "Tilt",
        "InvertedEraser",
        "BarrelButton",
        "Shortcuts",
    ]
    .map(|aspect| {
        serde_json::json!({"aspect":aspect,"result":"NotTested","method":"NotPerformed",
            "inputApi":"Unknown","evidenceSha256":null,"deviceModel":null})
    });
    let bytes = serde_json::to_vec(
        &serde_json::json!({"schema":1,"kind":"paint-compatibility-observations",
        "identity":image,"toolId":tool,"settingsDigest":settings,"observations":observations}),
    )
    .map_err(|_| SessionError::Invalid)?;
    if bytes.len() > 32 * 1024 {
        return Err(SessionError::Backpressure);
    }
    Ok(bytes)
}

#[uniffi::export]
impl RemoteCompatibilityObservation {
    pub fn receipt_bytes(&self) -> SessionResult<Vec<u8>> {
        #[cfg(windows)]
        {
            let owned = self.owned.lock().map_err(|_| SessionError::Worker)?;
            let owned = owned.as_ref().ok_or(SessionError::Closed)?;
            owned.current()?;
            Ok(owned.receipt.clone())
        }
        #[cfg(not(windows))]
        {
            Err(SessionError::RemoteUnavailable)
        }
    }
    pub fn is_current(&self) -> SessionResult<bool> {
        #[cfg(windows)]
        {
            let owned = self.owned.lock().map_err(|_| SessionError::Worker)?;
            let owned = owned.as_ref().ok_or(SessionError::Closed)?;
            owned.current()?;
            owned.verify_source()?;
            let actual = owned
                .helper
                .observe(&owned.target, owned.source.clone(), &owned.cancel)
                .map_err(failure)?;
            owned.verify_source()?;
            owned.current()?;
            Ok(actual == owned.observation)
        }
        #[cfg(not(windows))]
        {
            Err(SessionError::RemoteUnavailable)
        }
    }
    pub fn release_source(&self) -> SessionResult<()> {
        #[cfg(windows)]
        {
            self.owned.lock().map_err(|_| SessionError::Worker)?.take();
        }
        Ok(())
    }
}

#[uniffi::export]
impl LiveSession {
    /// Called only with a hash from the retained application runtime inventory.
    pub fn remote_configure_observation_helper(
        &self,
        path: String,
        sha256: String,
    ) -> SessionResult<()> {
        #[cfg(windows)]
        {
            if !self.remote.host {
                return Err(SessionError::Invalid);
            }
            let helper = Arc::new(
                vw_remote_host::editor_probe_runner::PaintObservationHelper::open(
                    std::path::Path::new(&path),
                    &sha256,
                )
                .map_err(failure)?,
            );
            let _operation = self
                .remote
                .operations
                .lock()
                .map_err(|_| SessionError::Worker)?;
            let mut s = self.remote.state.lock().map_err(|_| SessionError::Worker)?;
            if s.closed {
                return Err(SessionError::Closed);
            }
            s.runtime
                .as_mut()
                .ok_or(SessionError::RemoteUnavailable)?
                .observation = Some(helper);
            Ok(())
        }
        #[cfg(not(windows))]
        {
            let _ = (path, sha256);
            Err(SessionError::RemoteUnavailable)
        }
    }
    pub fn remote_observe_compatibility(
        &self,
        binding_json: String,
    ) -> SessionResult<Arc<RemoteCompatibilityObservation>> {
        #[cfg(windows)]
        {
            if !self.remote.host || binding_json.len() > 4096 {
                return Err(SessionError::Invalid);
            }
            let binding: Binding =
                serde_json::from_str(&binding_json).map_err(|_| SessionError::Invalid)?;
            let (target, identity, profile_digest, helper) = {
                let s = self.remote.state.lock().map_err(|_| SessionError::Worker)?;
                RemoteHub::current(&s)?;
                if s.host_retiring || s.binding.as_ref() != Some(&binding) {
                    return Err(SessionError::Closed);
                }
                let authority = s
                    .authority
                    .as_ref()
                    .ok_or(SessionError::RemoteUnavailable)?;
                if authority.identity.executable_name != "mspaint.exe" {
                    return Err(SessionError::RemoteUnavailable);
                }
                (
                    s.selected
                        .as_ref()
                        .ok_or(SessionError::Closed)?
                        .target
                        .clone(),
                    authority.identity.clone(),
                    authority.profile_digest.clone(),
                    s.runtime
                        .as_ref()
                        .and_then(|r| r.observation.clone())
                        .ok_or(SessionError::RemoteUnavailable)?,
                )
            };
            let cancel = Arc::new(vw_remote_host::process::Cancellation::default());
            {
                let mut registered = self
                    .remote
                    .cancellations
                    .lock()
                    .map_err(|_| SessionError::Worker)?;
                registered.retain(|v| v.strong_count() != 0);
                if registered.len() >= 8 {
                    return Err(SessionError::Backpressure);
                }
                registered.push(Arc::downgrade(&cancel));
            }
            let source = Arc::new(
                vw_remote_host::editor_source::SourceLease::open_paint_observation(
                    &target,
                    std::process::id(),
                    &identity,
                )
                .map_err(failure)?,
            );
            cancel.check().map_err(failure)?;
            let observation = helper
                .observe(&target, source.clone(), &cancel)
                .map_err(failure)?;
            let receipt = receipt(
                &identity,
                &profile_digest,
                &observation.tool_id,
                &observation.settings_digest,
            )?;
            let owned = Owned {
                hub: Arc::downgrade(&self.remote),
                binding,
                target,
                identity,
                profile_digest,
                source,
                cancel,
                helper,
                observation,
                receipt,
            };
            owned.verify_source()?;
            owned.current()?;
            Ok(Arc::new(RemoteCompatibilityObservation {
                owned: Mutex::new(Some(owned)),
            }))
        }
        #[cfg(not(windows))]
        {
            let _ = binding_json;
            Err(SessionError::RemoteUnavailable)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn query_receipt_never_claims_effect_or_physical_support() -> SessionResult<()> {
        let identity = vw_remote::profile::ImageIdentity {
            executable_name: "mspaint.exe".into(),
            executable_blake3: "a".repeat(64),
            executable_bytes: 128,
            file_version: None,
            package_full_name: Some("Paint_1.2.3.4_x64__test".into()),
            package_version: Some("1.2.3.4".into()),
        };
        let value: serde_json::Value = serde_json::from_slice(&receipt(
            &identity,
            &"b".repeat(64),
            "paint.brush",
            &"c".repeat(64),
        )?)
        .map_err(|_| SessionError::Invalid)?;
        let rows = value["observations"]
            .as_array()
            .ok_or(SessionError::Invalid)?;
        assert_eq!(rows.len(), 7);
        for row in rows {
            assert_eq!(row["result"], "NotTested");
            assert_eq!(row["method"], "NotPerformed");
            assert_eq!(row["inputApi"], "Unknown");
            assert!(row["evidenceSha256"].is_null());
            assert!(row["deviceModel"].is_null());
        }
        Ok(())
    }
}
