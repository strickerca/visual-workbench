//! Root-admitted packaged canvas/finite command profiles. Shape validation is
//! deliberately not admission; the local packaged SHA-256 inventory supplies it.
use crate::{Error, Rect, Result};
use serde::{Deserialize, Serialize};
pub mod catalog;
pub mod essential;
pub mod live;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageIdentity {
    pub executable_name: String,
    pub executable_blake3: String,
    pub executable_bytes: u64,
    pub file_version: Option<String>,
    pub package_full_name: Option<String>,
    pub package_version: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CanvasSelector {
    pub framework_id: String,
    pub class_name: String,
    pub automation_id: String,
    pub control_type: i32,
    pub require_keyboard_focus: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Action {
    pub action: u32,
    pub keys: Vec<u16>,
    pub native_batch_digest: String,
    pub effect_receipt_digest: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Profile {
    pub schema_version: u32,
    pub acceptance_receipt_digest: String,
    pub guard_lifecycle_receipt_digest: String,
    pub executable_name: String,
    pub executable_blake3: String,
    pub executable_bytes: u64,
    pub file_version: Option<String>,
    pub package_full_name: Option<String>,
    pub package_version: Option<String>,
    pub tool_id: String,
    pub settings_digest: String,
    pub canvas_selector: CanvasSelector,
    pub actions: Vec<Action>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackagedProfile {
    pub digest: String,
    pub json: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanvasProof {
    pub runtime_id_hash: String,
    pub process_id: u32,
    pub sampled_qpc_100ns: u64,
    pub class_name: String,
    pub control_type: i32,
    pub canvas_rect: Rect,
    pub profile_digest: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Authority {
    pub identity: ImageIdentity,
    pub profile_digest: String,
    pub verified_actions: Vec<u32>,
    pub focus: CanvasProof,
}
pub fn digest(v: &str) -> bool {
    v.len() == 64
        && v.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn text(v: &str, limit: usize, empty: bool) -> bool {
    (empty || !v.is_empty()) && v.len() <= limit && !v.chars().any(char::is_control)
}
impl Profile {
    pub fn identity(&self) -> ImageIdentity {
        ImageIdentity {
            executable_name: self.executable_name.clone(),
            executable_blake3: self.executable_blake3.clone(),
            executable_bytes: self.executable_bytes,
            file_version: self.file_version.clone(),
            package_full_name: self.package_full_name.clone(),
            package_version: self.package_version.clone(),
        }
    }
    pub fn validate(&self) -> Result<()> {
        let i = self.identity();
        let s = &self.canvas_selector;
        if self.schema_version != 1
            || !digest(&self.acceptance_receipt_digest)
            || !digest(&self.guard_lifecycle_receipt_digest)
            || !digest(&i.executable_blake3)
            || !text(&i.executable_name, 128, false)
            || i.executable_name.contains(['\\', '/'])
            || !(64..=256 * 1024 * 1024).contains(&i.executable_bytes)
            || !text(&self.tool_id, 128, false)
            || !digest(&self.settings_digest)
            || !s.require_keyboard_focus
            || !text(&s.framework_id, 128, false)
            || !text(&s.class_name, 256, false)
            || !text(&s.automation_id, 256, true)
            || !(50000..=50100).contains(&s.control_type)
            || self.actions.len() > 16
        {
            return Err(Error::Invalid);
        }
        let krita = i.executable_name.eq_ignore_ascii_case("krita.exe")
            && i.file_version.as_ref().is_some_and(|v| version(v));
        let paint = i.executable_name.eq_ignore_ascii_case("mspaint.exe")
            && i.package_full_name
                .as_ref()
                .is_some_and(|v| text(v, 2048, false))
            && i.package_version.as_ref().is_some_and(|v| version(v));
        if !krita && !paint {
            return Err(Error::Unavailable);
        }
        if i.file_version.as_ref().is_some_and(|v| !version(v))
            || i.package_full_name.is_none() != i.package_version.is_none()
        {
            return Err(Error::Invalid);
        }
        let mut seen = 0u32;
        for a in &self.actions {
            if !(1..=16).contains(&a.action)
                || seen & (1 << a.action) != 0
                || !digest(&a.native_batch_digest)
                || !digest(&a.effect_receipt_digest)
                || a.keys.is_empty()
                || a.keys.len() > 4
                || a.keys.iter().any(|k| !allowed_key(*k))
                || a.keys
                    .iter()
                    .enumerate()
                    .any(|(n, k)| a.keys[..n].contains(k))
                || (krita && a.action == 5)
                || (paint && matches!(a.action, 4 | 12))
            {
                return Err(Error::Invalid);
            }
            seen |= 1 << a.action;
        }
        Ok(())
    }
}
fn version(v: &str) -> bool {
    let p = v.split('.').collect::<Vec<_>>();
    p.len() == 4
        && p.iter().all(|p| {
            !p.is_empty()
                && p.len() <= 5
                && p.bytes().all(|c| c.is_ascii_digit())
                && p.parse::<u16>().is_ok()
        })
}
/// No Windows/system/global-navigation keys; an admitted finite route still
/// requires exact current native focus. The complete chord is released in reverse.
pub fn allowed_key(k: u16) -> bool {
    matches!(k,0x10|0x11|0x12|0x20|0x25..=0x28|0x30..=0x39|0x41..=0x5a|0x6b|0x6d|0xbb|0xbd|0xdb|0xdd)
}
pub fn key_batch_bytes(keys: &[u16]) -> Vec<u8> {
    let mut bytes = b"M4-PairedKeys-v1\0".to_vec();
    for k in keys {
        bytes.extend_from_slice(&k.to_le_bytes());
        bytes.push(0)
    }
    for k in keys.iter().rev() {
        bytes.extend_from_slice(&k.to_le_bytes());
        bytes.push(1)
    }
    bytes
}
