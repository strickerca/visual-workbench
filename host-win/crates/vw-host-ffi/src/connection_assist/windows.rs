//! Native adapters. Tests exercise their pure decisions through injected
//! backends; no test calls these process, routing or elevation functions.
mod elevation;
mod process;

use super::{
    ConnectionAssistError as Error, ConnectionRequest, Result,
    adb::AdbToolInfo,
    routes::{self, RouteFamily, RouteRow},
};
use std::{
    fs::{File, OpenOptions},
    io::Read,
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    ptr,
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
    NetworkManagement::IpHelper::{
        FreeMibTable, GetIpForwardTable2, GetIpInterfaceEntry, MIB_IPFORWARD_TABLE2,
        MIB_IPINTERFACE_ROW,
    },
    Networking::WinSock::{AF_INET, AF_INET6, AF_UNSPEC},
    Storage::FileSystem::FILE_SHARE_READ,
};

pub(super) use elevation::{elevate_metric, helper_main};

struct Handle(HANDLE);
impl Handle {
    fn checked(value: HANDLE) -> Result<Self> {
        if value.is_null() || value == INVALID_HANDLE_VALUE {
            Err(Error::WorkerUnavailable)
        } else {
            Ok(Self(value))
        }
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: this wrapper uniquely owns one successful Win32 handle.
        unsafe {
            CloseHandle(self.0);
        }
    }
}
fn wide(value: &std::ffi::OsStr) -> Result<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;
    let result: Vec<_> = value.encode_wide().collect();
    if result.is_empty() || result.len() > 32_000 || result.contains(&0) {
        return Err(Error::InvalidSelection);
    }
    Ok(result.into_iter().chain(Some(0)).collect())
}

pub(super) struct CheckedTool {
    pub info: AdbToolInfo,
    _lease: File,
}
pub(super) fn inspect_tool(path: &str, request: &ConnectionRequest) -> Result<CheckedTool> {
    super::adb::validate_path(path)?;
    // Selected tools must be local installed files. Do not execute UNC shares,
    // search PATH, substitute bundled tools or download a missing executable.
    if path.starts_with("\\\\") {
        return Err(Error::InvalidAdbTool);
    }
    request.check()?;
    let selected = Path::new(path);
    let metadata = std::fs::symlink_metadata(selected).map_err(|_| Error::InvalidAdbTool)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 64 * 1024 * 1024
    {
        return Err(Error::InvalidAdbTool);
    }
    let mut lease = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(selected)
        .map_err(|_| Error::InvalidAdbTool)?;
    let mut magic = [0; 2];
    lease
        .read_exact(&mut magic)
        .map_err(|_| Error::InvalidAdbTool)?;
    if magic != *b"MZ" {
        return Err(Error::InvalidAdbTool);
    }
    let output = process::run(
        selected,
        &["version".into()],
        request,
        Duration::from_secs(2),
    )?;
    if output.code != 0 || !valid_tool_version(&output.output) {
        return Err(Error::UnsupportedAdb);
    }
    Ok(CheckedTool {
        info: AdbToolInfo {
            protocol_version: 41,
        },
        _lease: lease,
    })
}
fn valid_tool_version(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes)
        .ok()
        .and_then(|s| s.lines().next())
        == Some("Android Debug Bridge version 1.0.41")
}

struct Table(*mut MIB_IPFORWARD_TABLE2);
impl Drop for Table {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: GetIpForwardTable2 returned this allocation.
            unsafe {
                FreeMibTable(self.0.cast());
            }
        }
    }
}
pub(super) fn native_family(family: RouteFamily) -> u16 {
    if family == RouteFamily::Ipv4 {
        AF_INET
    } else {
        AF_INET6
    }
}
pub(super) fn read_routes() -> Result<Vec<RouteRow>> {
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    let mut table = Table(ptr::null_mut());
    // SAFETY: valid out pointer, both documented address families requested.
    if unsafe { GetIpForwardTable2(AF_UNSPEC, &mut table.0) } != 0 || table.0.is_null() {
        return Err(Error::RouteUnavailable);
    }
    // SAFETY: successful query owns the initialized header and contiguous rows.
    let count = unsafe { (*table.0).NumEntries } as usize;
    if count > 4096 {
        return Err(Error::RouteUnavailable);
    }
    // SAFETY: bounded NumEntries is the length allocated by Windows; Table lives
    // through the iteration and frees its storage only afterward.
    let rows = unsafe { std::slice::from_raw_parts(ptr::addr_of!((*table.0).Table).cast(), count) };
    let mut result = Vec::new();
    for row in rows {
        if std::time::Instant::now() >= deadline {
            return Err(Error::RouteUnavailable);
        }
        let row: &windows_sys::Win32::NetworkManagement::IpHelper::MIB_IPFORWARD_ROW2 = row;
        if row.DestinationPrefix.PrefixLength != 0 || row.Loopback || row.ValidLifetime == 0 {
            continue;
        }
        // SAFETY: common discriminant of SOCKADDR_INET.
        let family = match unsafe { row.DestinationPrefix.Prefix.si_family } {
            AF_INET => RouteFamily::Ipv4,
            AF_INET6 => RouteFamily::Ipv6,
            _ => continue,
        };
        let mut interface = MIB_IPINTERFACE_ROW {
            Family: native_family(family),
            InterfaceLuid: row.InterfaceLuid,
            InterfaceIndex: row.InterfaceIndex,
            ..Default::default()
        };
        // SAFETY: all identifying fields are initialized, API fills the row.
        if unsafe { GetIpInterfaceEntry(&mut interface) } != 0 {
            return Err(Error::RouteUnavailable);
        }
        if !interface.Connected || interface.DisableDefaultRoutes {
            continue;
        }
        let mut next_hop = blake3::Hasher::new();
        // SAFETY: family selects the active sockaddr variant. Hash only defined
        // fields, never struct padding, names or addresses into logs/evidence.
        unsafe {
            if row.NextHop.si_family != native_family(family) {
                return Err(Error::RouteUnavailable);
            }
            match family {
                RouteFamily::Ipv4 => {
                    next_hop.update(&row.NextHop.Ipv4.sin_addr.S_un.S_addr.to_le_bytes());
                }
                RouteFamily::Ipv6 => {
                    next_hop.update(&row.NextHop.Ipv6.sin6_addr.u.Byte);
                    next_hop.update(&row.NextHop.Ipv6.Anonymous.sin6_scope_id.to_le_bytes());
                }
            }
        }
        result.push(RouteRow {
            family,
            index: row.InterfaceIndex,
            // SAFETY: Value is the documented whole-LUID view of the union.
            luid: unsafe { row.InterfaceLuid.Value },
            route_metric: row.Metric,
            metric: interface.Metric,
            automatic: interface.UseAutomaticMetric,
            next_hop_hash: *next_hop.finalize().as_bytes(),
        });
    }
    routes::snapshot_hash(&result)?;
    Ok(result)
}

fn helper_path() -> Result<PathBuf> {
    use windows_sys::Win32::System::LibraryLoader::{
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
        GetModuleFileNameW, GetModuleHandleExW,
    };
    let mut module = ptr::null_mut();
    // SAFETY: FROM_ADDRESS makes lpModuleName an address in this DLL. The ref
    // count is unchanged; it is not an owned handle and must not be closed.
    if unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            helper_path as *const () as *const u16,
            &mut module,
        )
    } == 0
    {
        return Err(Error::HelperUnavailable);
    }
    let mut buffer = vec![0u16; 32_768];
    // SAFETY: writable buffer of the supplied length and valid module handle.
    let length =
        unsafe { GetModuleFileNameW(module, buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    if length == 0 || length >= buffer.len() {
        return Err(Error::HelperUnavailable);
    }
    use std::os::windows::ffi::OsStringExt;
    let library = PathBuf::from(std::ffi::OsString::from_wide(&buffer[..length]));
    let helper = library
        .parent()
        .ok_or(Error::HelperUnavailable)?
        .join("vw-connection-helper.exe");
    let metadata = std::fs::symlink_metadata(&helper).map_err(|_| Error::HelperUnavailable)?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > 128 * 1024 * 1024
    {
        return Err(Error::HelperUnavailable);
    }
    Ok(helper)
}

#[cfg(test)]
mod tests {
    use super::valid_tool_version;
    #[test]
    fn local_version_is_exact_and_never_reuses_other_output() {
        assert!(valid_tool_version(
            b"Android Debug Bridge version 1.0.41\r\nVersion x\r\nInstalled as private-path\r\n"
        ));
        assert!(!valid_tool_version(
            b"Android Debug Bridge version 1.0.39\n"
        ));
        assert!(!valid_tool_version(
            b"arbitrary prefix\nAndroid Debug Bridge version 1.0.41\n"
        ));
    }
}
