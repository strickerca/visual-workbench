use crate::{
    DpiInfo, HostError, HostResult, WindowDpiInfo,
    worker::{Backend, RequestContext},
};
pub(crate) fn set_process_per_monitor_v2() -> HostResult<crate::ProcessDpiInfo> {
    Err(HostError::UnsupportedPlatform)
}
pub(crate) struct Native;
impl Native {
    pub(crate) fn new() -> HostResult<Self> {
        Err(HostError::UnsupportedPlatform)
    }
}
impl Backend for Native {
    fn dpi_at_point(&mut self, _: i32, _: i32) -> HostResult<DpiInfo> {
        Err(HostError::UnsupportedPlatform)
    }
    fn window_dpi_info(&mut self, _: u64) -> HostResult<WindowDpiInfo> {
        Err(HostError::UnsupportedPlatform)
    }
    fn publish_png(&mut self, _: &[u8], _: &RequestContext) -> HostResult<u32> {
        Err(HostError::UnsupportedPlatform)
    }
    fn publish_image(&mut self, _: &[u8], _: &[u8], _: &RequestContext) -> HostResult<u32> {
        Err(HostError::UnsupportedPlatform)
    }
    fn read_image(
        &mut self,
        _: crate::ClipboardReadFormat,
        _: &RequestContext,
    ) -> HostResult<crate::worker::ClipboardSnapshot> {
        Err(HostError::UnsupportedPlatform)
    }
    fn pump_messages(&mut self) {}
}
