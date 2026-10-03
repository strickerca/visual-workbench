//! Native read-only route enumeration. No child shells, endpoint logging,
//! administrator prompts or metric changes are performed.
use std::ptr;
use vw_net::{
    pairing::{PairingError, Result},
    route::{AddressFamily, DefaultRoute, RouteProvider},
};
use windows_sys::Win32::{
    NetworkManagement::IpHelper::{
        FreeMibTable, GetIpForwardTable2, GetIpInterfaceEntry, MIB_IPFORWARD_TABLE2,
        MIB_IPINTERFACE_ROW,
    },
    Networking::WinSock::{AF_INET, AF_INET6, AF_UNSPEC},
};

pub struct WindowsRoutes;
struct RouteTable(*mut MIB_IPFORWARD_TABLE2);
impl Drop for RouteTable {
    fn drop(&mut self) {
        if !self.0.is_null() {
            /* SAFETY: GetIpForwardTable2 allocated this table. */
            unsafe {
                FreeMibTable(self.0.cast());
            }
        }
    }
}
impl RouteProvider for WindowsRoutes {
    fn default_routes(&self) -> Result<Vec<DefaultRoute>> {
        let mut table = RouteTable(ptr::null_mut());
        // SAFETY: valid output pointer, AF_UNSPEC asks for both IP families.
        if unsafe { GetIpForwardTable2(AF_UNSPEC, &mut table.0) } != 0 || table.0.is_null() {
            return Err(PairingError::Connection);
        }
        // SAFETY: successful API initialized the table header.
        let count = unsafe { (*table.0).NumEntries } as usize;
        if count > 4096 {
            return Err(PairingError::Capacity);
        }
        // SAFETY: Windows allocates NumEntries contiguous rows beyond the header.
        // Count is bounded before constructing the slice and table outlives it.
        let rows =
            unsafe { std::slice::from_raw_parts(ptr::addr_of!((*table.0).Table).cast(), count) };
        let mut result = Vec::new();
        for row in rows {
            let row: &windows_sys::Win32::NetworkManagement::IpHelper::MIB_IPFORWARD_ROW2 = row;
            if row.DestinationPrefix.PrefixLength != 0 || row.Loopback || row.ValidLifetime == 0 {
                continue;
            }
            // SAFETY: SOCKADDR_INET family is its common union discriminant.
            let native_family = unsafe { row.DestinationPrefix.Prefix.si_family };
            let family = match native_family {
                AF_INET => AddressFamily::Ipv4,
                AF_INET6 => AddressFamily::Ipv6,
                _ => continue,
            };
            let mut interface = MIB_IPINTERFACE_ROW {
                Family: native_family,
                InterfaceLuid: row.InterfaceLuid,
                InterfaceIndex: row.InterfaceIndex,
                ..Default::default()
            };
            // SAFETY: documented identifying fields are initialized; API fills row.
            if unsafe { GetIpInterfaceEntry(&mut interface) } != 0 {
                return Err(PairingError::Connection);
            }
            if !interface.Connected || interface.DisableDefaultRoutes {
                continue;
            }
            result.push(DefaultRoute {
                interface_index: row.InterfaceIndex,
                family,
                route_metric: row.Metric,
                interface_metric: interface.Metric,
                automatic_metric: interface.UseAutomaticMetric,
            });
        }
        Ok(result)
    }
}
