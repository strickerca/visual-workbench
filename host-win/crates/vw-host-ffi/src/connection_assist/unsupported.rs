use super::{
    ConnectionAssistError as Error, ConnectionRequest, Result,
    adb::AdbToolInfo,
    routes::{Plan, RouteRow},
};
pub(super) struct CheckedTool {
    pub info: AdbToolInfo,
}
pub(super) fn inspect_tool(_: &str, _: &ConnectionRequest) -> Result<CheckedTool> {
    Err(Error::Unsupported)
}
pub(super) fn read_routes() -> Result<Vec<RouteRow>> {
    Err(Error::Unsupported)
}
pub(super) fn elevate_metric(_: &Plan, _: &ConnectionRequest) -> Result<()> {
    Err(Error::Unsupported)
}
pub(super) fn helper_main() -> i32 {
    20
}
