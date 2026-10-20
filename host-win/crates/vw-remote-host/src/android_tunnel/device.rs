use super::{Error, Result};

const MAX_LIST_BYTES: usize = 64 * 1024;
const MAX_DEVICES: usize = 32;

/// A request chosen by the owner UI. A model or transport is never guessed.
#[derive(Clone, Debug)]
pub struct UsbSelection {
    model: String,
    transport_id: u32,
}

impl UsbSelection {
    pub fn new(model: String, transport_id: u32) -> Result<Self> {
        if model.is_empty()
            || model.len() > 64
            || !model
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
            || transport_id == 0
        {
            return Err(Error::Invalid);
        }
        Ok(Self {
            model,
            transport_id,
        })
    }
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct SelectedDevice {
    pub(super) serial: String,
    pub(super) model: String,
    pub(super) transport_id: u32,
}

/// Opaque per-session binding; no serial or transport is exposed to consumers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeviceToken(pub(crate) [u8; 16]);

impl DeviceToken {
    pub fn same_binding(self, other: Self) -> bool {
        self.0 == other.0
    }
}

pub(crate) fn select(bytes: &[u8], chosen: &UsbSelection) -> Result<SelectedDevice> {
    if bytes.len() > MAX_LIST_BYTES {
        return Err(Error::Limit);
    }
    let listing = std::str::from_utf8(bytes).map_err(|_| Error::Invalid)?;
    let mut rows = listing.lines();
    if rows.next() != Some("List of devices attached") {
        return Err(Error::Invalid);
    }
    let mut count = 0;
    let mut match_row = None;
    for line in rows.filter(|line| !line.is_empty()) {
        count += 1;
        if count > MAX_DEVICES {
            return Err(Error::Limit);
        }
        let (serial, rest) = line.split_once('\t').ok_or(Error::Invalid)?;
        // A USB serial cannot be an adb TCP address, injected option or shell
        // fragment. It remains private even in error diagnostics.
        if serial.is_empty()
            || serial.len() > 128
            || !serial
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        {
            return Err(Error::Invalid);
        }
        let mut words = rest.split_ascii_whitespace();
        let state = words.next().ok_or(Error::Invalid)?;
        let mut model = None;
        let mut transport = None;
        let mut usb = false;
        for word in words {
            if word.starts_with("usb:") {
                usb = true;
            } else if let Some(value) = word.strip_prefix("model:") {
                if model.replace(value).is_some() {
                    return Err(Error::Invalid);
                }
            } else if let Some(value) = word.strip_prefix("transport_id:") {
                let id = value.parse::<u32>().map_err(|_| Error::Invalid)?;
                if transport.replace(id).is_some() {
                    return Err(Error::Invalid);
                }
            }
        }
        if transport != Some(chosen.transport_id) {
            continue;
        }
        if state == "unauthorized" {
            return Err(Error::UnauthorizedDevice);
        }
        if state != "device" || !usb || model != Some(chosen.model.as_str()) {
            return Err(Error::Consent);
        }
        let row = SelectedDevice {
            serial: serial.to_owned(),
            model: chosen.model.clone(),
            transport_id: chosen.transport_id,
        };
        if match_row.replace(row).is_some() {
            return Err(Error::AmbiguousDevice);
        }
    }
    match_row.ok_or(Error::Consent)
}

pub(crate) fn still_selected(actual: &SelectedDevice, new_listing: &[u8]) -> Result<()> {
    let selection = UsbSelection::new(actual.model.clone(), actual.transport_id)?;
    if select(new_listing, &selection)? != *actual {
        return Err(Error::Stale);
    }
    Ok(())
}
