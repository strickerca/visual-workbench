//! Independent wire encoding for SudoVDA protocol 0.2.1, pinned in upstream.json.
//! The separate driver source/license never links into this proprietary client.

pub const ADD: u32 = 0x0022_2000;
pub const REMOVE: u32 = 0x0022_2004;
pub const WATCHDOG: u32 = 0x0022_200c;
pub const PING: u32 = 0x0022_2220;
pub const VERSION: u32 = 0x0022_23fc;

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("Only the two planned 60 Hz phone modes are permitted")]
    Mode,
    #[error("The driver returned an unexpected protocol version or buffer length")]
    Response,
    #[error("The shared driver watchdog must be enabled with a three-second timeout")]
    Watchdog,
    #[error("A zero monitor GUID is not permitted")]
    Guid,
}

#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct Mode {
    pub width: u32,
    pub height: u32,
    pub refresh_millihertz: u32,
}

pub const MODES: [Mode; 2] = [
    Mode {
        width: 3088,
        height: 1440,
        refresh_millihertz: 60_000,
    },
    Mode {
        width: 1440,
        height: 3088,
        refresh_millihertz: 60_000,
    },
];

/// A Windows GUID's native little-endian field bytes, never a user/device serial.
pub fn add_request(mode: Mode, guid: [u8; 16]) -> Result<[u8; 56], ProtocolError> {
    if !MODES.iter().any(|m| {
        m.width == mode.width
            && m.height == mode.height
            && m.refresh_millihertz == mode.refresh_millihertz
    }) {
        return Err(ProtocolError::Mode);
    }
    if guid == [0; 16] {
        return Err(ProtocolError::Guid);
    }
    let mut bytes = [0; 56];
    bytes[0..4].copy_from_slice(&mode.width.to_le_bytes());
    bytes[4..8].copy_from_slice(&mode.height.to_le_bytes());
    bytes[8..12].copy_from_slice(&mode.refresh_millihertz.to_le_bytes());
    bytes[12..28].copy_from_slice(&guid);
    // Upstream calls strlen on both char[14] fields. Keep the last byte NUL.
    bytes[28..36].copy_from_slice(b"VW Probe");
    bytes[42..51].copy_from_slice(b"SYNTHETIC");
    Ok(bytes)
}

pub fn check_version(bytes: &[u8]) -> Result<(), ProtocolError> {
    // Upstream labels this exact source protocol TestBuild=true; never ignore it.
    if bytes == [0, 2, 1, 1] {
        Ok(())
    } else {
        Err(ProtocolError::Response)
    }
}

pub fn check_watchdog(bytes: &[u8]) -> Result<(), ProtocolError> {
    if bytes.len() != 8 {
        return Err(ProtocolError::Response);
    }
    if bytes[..4] != 3_u32.to_le_bytes() {
        return Err(ProtocolError::Watchdog);
    }
    let countdown = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    if countdown > 3 {
        return Err(ProtocolError::Response);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct Target {
    pub adapter_low: u32,
    pub adapter_high: i32,
    pub target: u32,
}

pub fn add_response(bytes: &[u8]) -> Result<Target, ProtocolError> {
    let fields: &[u8; 12] = bytes.try_into().map_err(|_| ProtocolError::Response)?;
    Ok(Target {
        adapter_low: u32::from_le_bytes([fields[0], fields[1], fields[2], fields[3]]),
        adapter_high: i32::from_le_bytes([fields[4], fields[5], fields[6], fields[7]]),
        target: u32::from_le_bytes([fields[8], fields[9], fields[10], fields[11]]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_wire_layout_and_terminated_names() -> Result<(), ProtocolError> {
        for mode in MODES {
            let guid = [0xa5; 16];
            let bytes = add_request(mode, guid)?;
            assert_eq!(bytes.len(), 56);
            assert_eq!(&bytes[12..28], &guid);
            assert_eq!(&bytes[8..12], &60_000_u32.to_le_bytes());
            assert_eq!(bytes[41], 0);
            assert_eq!(bytes[55], 0);
            assert_eq!(&bytes[28..36], b"VW Probe");
        }
        Ok(())
    }
    #[test]
    fn refuses_unplanned_modes_and_zero_identity() {
        assert!(
            add_request(
                Mode {
                    width: u32::MAX,
                    ..MODES[0]
                },
                [1; 16]
            )
            .is_err()
        );
        assert!(
            add_request(
                Mode {
                    refresh_millihertz: 60,
                    ..MODES[0]
                },
                [1; 16]
            )
            .is_err()
        );
        assert!(add_request(MODES[0], [0; 16]).is_err());
    }
    #[test]
    fn rejects_protocol_drift_disabled_watchdog_and_short_output() -> Result<(), ProtocolError> {
        check_version(&[0, 2, 1, 1])?;
        assert!(check_version(&[0, 2, 1, 0]).is_err());
        assert!(check_version(&[0, 2, 1]).is_err());
        check_watchdog(&[3, 0, 0, 0, 2, 0, 0, 0])?;
        assert!(check_watchdog(&[0; 8]).is_err());
        assert!(check_watchdog(&[3, 0, 0, 0, 4, 0, 0, 0]).is_err());
        assert!(add_response(&[0; 11]).is_err());
        let target = add_response(&[1, 0, 0, 0, 255, 255, 255, 255, 2, 0, 0, 0])?;
        assert_eq!(target.adapter_low, 1);
        assert_eq!(target.adapter_high, -1);
        assert_eq!(target.target, 2);
        Ok(())
    }
}
