//! Actual admitted Windows producer. Other platforms never fabricate a mirror.
use super::*;
use contract::{OpenProvider, Stop};

pub(super) fn available() -> bool {
    #[cfg(windows)]
    {
        vw_remote_host::android_tunnel_root::package_available()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(not(windows))]
pub(super) fn factory(
    _options: AndroidTunnelOptions,
    _session: [u8; 16],
    _generation: u64,
) -> (OpenProvider, Arc<Stop>) {
    (
        Box::new(|_| Err(AndroidTunnelError::RuntimeUnavailable)),
        Stop::new(Arc::new(|| {})),
    )
}

#[cfg(windows)]
pub(super) fn factory(
    options: AndroidTunnelOptions,
    session: [u8; 16],
    generation: u64,
) -> (OpenProvider, Arc<Stop>) {
    use vw_remote_host::android_tunnel_root as native;
    let cancel = Arc::new(native::Cancellation::default());
    let signal = cancel.clone();
    let stop = Stop::new(Arc::new(move || signal.cancel()));
    let open: OpenProvider = Box::new(move |_| {
        let opened = native::LocalTunnelOwner::open(
            options.runtime_directory.into(),
            native::OwnerAdb::select(options.owner_adb_path.into()).map_err(map)?,
            native::UsbSelection::new(options.selected_model, options.selected_transport_id)
                .map_err(map)?,
            session,
            generation,
            &cancel,
        );
        match opened {
            Ok(owner) => Ok(Box::new(WindowsProvider { owner, cancel })),
            Err(native::Error::RetirementPending) => {
                Ok(Box::new(RetiringProvider::new(Box::new(|| {
                    // Raw F keeps the exact unresolved runner/IO/process in its
                    // exclusive registry. Retain its concrete retry route in this
                    // charged actor until every raw owner actually retires.
                    if native::retry_retirements().map_err(map)? == 0 {
                        Ok(())
                    } else {
                        Err(AndroidTunnelError::RetirementPending)
                    }
                }))))
            }
            Err(error) => Err(map(error)),
        }
    });
    (open, stop)
}

#[cfg(any(windows, test))]
pub(super) struct RetiringProvider {
    retry: Box<dyn FnMut() -> AndroidTunnelResult<()> + Send>,
}
#[cfg(any(windows, test))]
impl RetiringProvider {
    pub(super) fn new(retry: Box<dyn FnMut() -> AndroidTunnelResult<()> + Send>) -> Self {
        Self { retry }
    }
}
#[cfg(any(windows, test))]
impl contract::Provider for RetiringProvider {
    fn state(&mut self) -> AndroidTunnelResult<AndroidTunnelState> {
        Err(AndroidTunnelError::RetirementPending)
    }
    fn begin_confirmation(&mut self) -> AndroidTunnelResult<AndroidTunnelTarget> {
        Err(AndroidTunnelError::RetirementPending)
    }
    fn confirm(&mut self, _target: AndroidTunnelTarget) -> AndroidTunnelResult<AndroidTunnelState> {
        Err(AndroidTunnelError::RetirementPending)
    }
    fn cancel_confirmation(&mut self) -> AndroidTunnelResult<()> {
        Err(AndroidTunnelError::RetirementPending)
    }
    fn revoke_control(&mut self) -> AndroidTunnelResult<AndroidTunnelState> {
        Err(AndroidTunnelError::RetirementPending)
    }
    fn capture(&mut self) -> AndroidTunnelResult<contract::Captured> {
        Err(AndroidTunnelError::RetirementPending)
    }
    fn check_source(
        &mut self,
        _source: AndroidTunnelSource,
        _revision: u64,
    ) -> AndroidTunnelResult<()> {
        Err(AndroidTunnelError::RetirementPending)
    }
    fn close(&mut self) -> AndroidTunnelResult<()> {
        (self.retry)()
    }
}

#[cfg(windows)]
fn map(error: vw_remote_host::android_tunnel_root::Error) -> AndroidTunnelError {
    use vw_remote_host::android_tunnel_root::Error as E;
    match error {
        E::Invalid => AndroidTunnelError::Invalid,
        E::Consent => AndroidTunnelError::Consent,
        E::AmbiguousDevice => AndroidTunnelError::AmbiguousDevice,
        E::UnauthorizedDevice => AndroidTunnelError::UnauthorizedDevice,
        E::RuntimeUnavailable => AndroidTunnelError::RuntimeUnavailable,
        E::FeedbackLoop => AndroidTunnelError::FeedbackLoop,
        E::Stale => AndroidTunnelError::Stale,
        E::Limit => AndroidTunnelError::Limit,
        E::Cancelled => AndroidTunnelError::Cancelled,
        E::Timeout => AndroidTunnelError::Timeout,
        E::Storage => AndroidTunnelError::Storage,
        E::Platform => AndroidTunnelError::Platform,
        E::RetirementPending => AndroidTunnelError::RetirementPending,
    }
}

#[cfg(windows)]
struct WindowsProvider {
    owner: vw_remote_host::android_tunnel_root::LocalTunnelOwner,
    cancel: Arc<vw_remote_host::android_tunnel_root::Cancellation>,
}
#[cfg(windows)]
impl contract::Provider for WindowsProvider {
    fn state(&mut self) -> AndroidTunnelResult<AndroidTunnelState> {
        use vw_remote_host::android_tunnel_root::InteractionState as S;
        let (mode, mirror_revision) = match self.owner.interaction_state() {
            S::ViewOnly { mirror_revision } => (AndroidTunnelMode::ViewOnly, mirror_revision),
            S::OwnerControl { mirror_revision } => {
                (AndroidTunnelMode::OwnerControl, mirror_revision)
            }
            S::RetirementPending => (AndroidTunnelMode::RetirementPending, 0),
            S::Closed => (AndroidTunnelMode::Closed, 0),
        };
        Ok(AndroidTunnelState {
            mode,
            mirror_revision,
        })
    }
    fn begin_confirmation(&mut self) -> AndroidTunnelResult<AndroidTunnelTarget> {
        let target = self
            .owner
            .begin_control_confirmation(&self.cancel)
            .map_err(map)?;
        Ok(AndroidTunnelTarget {
            model: target.model().to_owned(),
            transport_id: target.transport_id(),
            mirror_revision: self.owner.mirror_revision(),
        })
    }
    fn confirm(&mut self, target: AndroidTunnelTarget) -> AndroidTunnelResult<AndroidTunnelState> {
        if target.mirror_revision != self.owner.mirror_revision() {
            self.owner.cancel_control_confirmation();
            return Err(AndroidTunnelError::Stale);
        }
        self.owner
            .confirm_control(&target.model, target.transport_id, &self.cancel)
            .map_err(map)?;
        self.state()
    }
    fn cancel_confirmation(&mut self) -> AndroidTunnelResult<()> {
        self.owner.cancel_control_confirmation();
        Ok(())
    }
    fn revoke_control(&mut self) -> AndroidTunnelResult<AndroidTunnelState> {
        self.owner.revoke_control().map_err(map)?;
        self.state()
    }
    fn capture(&mut self) -> AndroidTunnelResult<contract::Captured> {
        use vw_remote_host::android_tunnel_root::Hierarchy;
        let pair = self.owner.capture_pair(&self.cancel).map_err(map)?;
        let (hierarchy, hierarchy_age_ms, elements) = match pair.hierarchy {
            Hierarchy::Missing => (AndroidTunnelHierarchy::Missing, None, Vec::new()),
            Hierarchy::Stale { age_ms } => {
                (AndroidTunnelHierarchy::Stale, Some(age_ms), Vec::new())
            }
            Hierarchy::Present { age_ms, elements } => (
                AndroidTunnelHierarchy::Present,
                Some(age_ms),
                elements
                    .into_iter()
                    .map(|e| AndroidTunnelElement {
                        left: e.bounds[0],
                        top: e.bounds[1],
                        right: e.bounds[2],
                        bottom: e.bounds[3],
                        class_name: e.class,
                        resource_id: e.resource_id,
                        description: e.description,
                        text: e.text,
                    })
                    .collect(),
            ),
        };
        Ok(contract::Captured {
            png: pair.image.encoded().to_vec(),
            width: pair.image.width,
            height: pair.image.height,
            image_started_ms: pair.image_started_ms,
            image_finished_ms: pair.image_finished_ms,
            image_finished_timestamp_ns: pair.image_finished_timestamp_ns,
            hierarchy,
            hierarchy_age_ms,
            hierarchy_started_ms: pair.hierarchy_started_ms,
            hierarchy_finished_ms: pair.hierarchy_finished_ms,
            elements,
        })
    }
    fn check_source(
        &mut self,
        source: AndroidTunnelSource,
        revision: u64,
    ) -> AndroidTunnelResult<()> {
        use vw_remote_host::android_tunnel_root::{CaptureSource as S, PhysicalRect};
        let source = match source {
            AndroidTunnelSource::Window { window } => S::Window {
                hwnd: isize::try_from(window).map_err(|_| AndroidTunnelError::Invalid)?,
            },
            AndroidTunnelSource::Monitor { monitor } => S::Monitor {
                monitor: isize::try_from(monitor).map_err(|_| AndroidTunnelError::Invalid)?,
            },
            AndroidTunnelSource::Region {
                monitor,
                x,
                y,
                width,
                height,
            } => S::Region {
                monitor: isize::try_from(monitor).map_err(|_| AndroidTunnelError::Invalid)?,
                rect: PhysicalRect {
                    left: x,
                    top: y,
                    right: i32::try_from(i64::from(x) + i64::from(width))
                        .map_err(|_| AndroidTunnelError::Invalid)?,
                    bottom: i32::try_from(i64::from(y) + i64::from(height))
                        .map_err(|_| AndroidTunnelError::Invalid)?,
                },
            },
        };
        self.owner.check_source(source, revision).map_err(map)
    }
    fn close(&mut self) -> AndroidTunnelResult<()> {
        self.owner.close().map_err(map)
    }
}
