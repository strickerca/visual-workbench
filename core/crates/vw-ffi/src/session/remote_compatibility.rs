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
    context: ObservationContext,
    source: Arc<vw_remote_host::editor_source::SourceLease>,
    cancel: Arc<vw_remote_host::process::Cancellation>,
    observation: vw_remote_host::editor_probe_runner::PaintControlsObservation,
    receipt: Vec<u8>,
}
#[cfg(windows)]
struct ObservationContext {
    local_epoch: u64,
    source_epoch: Option<u64>,
    grant_revision: u64,
    binding: Binding,
    target: Target,
    identity: vw_remote::profile::ImageIdentity,
    profile_digest: String,
    helper: Arc<vw_remote_host::editor_probe_runner::PaintObservationHelper>,
}

#[cfg(windows)]
impl ObservationContext {
    fn current(&self, hub: &RemoteHub) -> SessionResult<()> {
        let s = hub.state.lock().map_err(|_| SessionError::Worker)?;
        RemoteHub::current(&s)?;
        if s.host_retiring
            || s.peer_retiring
            || s.epoch != self.local_epoch
            || s.source_epoch != self.source_epoch
            || s.source_epoch != Some(self.binding.scope.connection_epoch)
            || s.grant_revision != self.grant_revision
            || s.binding.as_ref() != Some(&self.binding)
            || s.selected.as_ref().map(|v| &v.target) != Some(&self.target)
            || s.selected.as_ref().map(|v| &v.scope) != Some(&self.binding.scope)
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
}
#[cfg(windows)]
impl Owned {
    fn current(&self) -> SessionResult<()> {
        self.cancel.check().map_err(failure)?;
        let hub = self.hub.upgrade().ok_or(SessionError::Closed)?;
        self.context.current(&hub)
    }
    fn verify_source(&self) -> SessionResult<()> {
        self.source
            .verify_observation(&self.context.target, std::process::id())
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
                .context
                .helper
                .observe(&owned.context.target, owned.source.clone(), &owned.cancel)
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
            let context = {
                let s = self.remote.state.lock().map_err(|_| SessionError::Worker)?;
                RemoteHub::current(&s)?;
                if s.host_retiring || s.peer_retiring || s.binding.as_ref() != Some(&binding) {
                    return Err(SessionError::Closed);
                }
                let authority = s
                    .authority
                    .as_ref()
                    .ok_or(SessionError::RemoteUnavailable)?;
                if authority.identity.executable_name != "mspaint.exe" {
                    return Err(SessionError::RemoteUnavailable);
                }
                ObservationContext {
                    local_epoch: s.epoch,
                    source_epoch: s.source_epoch,
                    grant_revision: s.grant_revision,
                    binding,
                    target: s
                        .selected
                        .as_ref()
                        .ok_or(SessionError::Closed)?
                        .target
                        .clone(),
                    identity: authority.identity.clone(),
                    profile_digest: authority.profile_digest.clone(),
                    helper: s
                        .runtime
                        .as_ref()
                        .and_then(|r| r.observation.clone())
                        .ok_or(SessionError::RemoteUnavailable)?,
                }
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
            context.current(&self.remote)?;
            let source = Arc::new(
                vw_remote_host::editor_source::SourceLease::open_paint_observation(
                    &context.target,
                    std::process::id(),
                    &context.identity,
                )
                .map_err(failure)?,
            );
            cancel.check().map_err(failure)?;
            context.current(&self.remote)?;
            let observation = context
                .helper
                .observe(&context.target, source.clone(), &cancel)
                .map_err(failure)?;
            let receipt = receipt(
                &context.identity,
                &context.profile_digest,
                &observation.tool_id,
                &observation.settings_digest,
            )?;
            let owned = Owned {
                hub: Arc::downgrade(&self.remote),
                context,
                source,
                cancel,
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
