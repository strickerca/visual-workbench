//! Root-admitted packaged canvas/finite command profiles. Shape validation is
//! deliberately not admission; the local packaged SHA-256 inventory supplies it.
use crate::{Error, Rect, Result, Target};
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
#[serde(deny_unknown_fields)]
pub struct CanvasProof {
    pub runtime_id_hash: String,
    pub process_id: u32,
    pub sampled_qpc_100ns: u64,
    pub class_name: String,
    pub control_type: i32,
    pub canvas_rect: Rect,
    pub profile_digest: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityRoutes {
    Legacy,
    Essential,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(try_from = "AuthorityFields")]
pub struct Authority {
    pub identity: ImageIdentity,
    pub profile_digest: String,
    pub verified_actions: Vec<u32>,
    pub routes: AuthorityRoutes,
    /// Present only after an actual focused-canvas query. Initial grants and
    /// legacy keyboard routes retain this stronger requirement.
    pub focus: Option<CanvasProof>,
    /// Essential click/wheel routes may retain current selected-canvas identity
    /// after their toolbar/numeric destination takes focus. This is not focus
    /// proof and never permits arbitrary keyboard input.
    pub selected_canvas: Option<CanvasProof>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthorityFields {
    identity: ImageIdentity,
    profile_digest: String,
    verified_actions: Vec<u32>,
    routes: AuthorityRoutes,
    #[serde(deserialize_with = "required_canvas_field")]
    focus: Option<CanvasProof>,
    #[serde(deserialize_with = "required_canvas_field")]
    selected_canvas: Option<CanvasProof>,
}
fn required_canvas_field<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<CanvasProof>, D::Error> {
    Option::<CanvasProof>::deserialize(deserializer)
}
impl TryFrom<AuthorityFields> for Authority {
    type Error = Error;
    fn try_from(value: AuthorityFields) -> Result<Self> {
        let result = Self {
            identity: value.identity,
            profile_digest: value.profile_digest,
            verified_actions: value.verified_actions,
            routes: value.routes,
            focus: value.focus,
            selected_canvas: value.selected_canvas,
        };
        result.validate()?;
        Ok(result)
    }
}
impl Authority {
    /// Exactly one native observation variant is required, including on JSON
    /// input. Missing, dual and unknown variants cannot become permission.
    pub fn canvas(&self) -> Result<&CanvasProof> {
        match (&self.focus, &self.selected_canvas) {
            (Some(value), None) | (None, Some(value)) => Ok(value),
            _ => Err(Error::Invalid),
        }
    }
    pub fn focused_canvas(&self) -> Result<&CanvasProof> {
        self.validate()?;
        self.focus.as_ref().ok_or(Error::Ungranted)
    }
    pub fn canvas_for_target(&self, target: &Target) -> Result<&CanvasProof> {
        self.validate()?;
        target.validate()?;
        let canvas = self.canvas()?;
        if canvas.process_id != target.process_id || !canvas.canvas_rect.inside(target.client_rect)
        {
            return Err(Error::Invalid);
        }
        Ok(canvas)
    }
    pub fn validate(&self) -> Result<()> {
        let canvas = self.canvas()?;
        if !digest(&self.profile_digest)
            || canvas.profile_digest != self.profile_digest
            || !digest(&canvas.runtime_id_hash)
            || canvas.process_id == 0
            || canvas.sampled_qpc_100ns == 0
            || !text(&canvas.class_name, 256, false)
            || !(50000..=50100).contains(&canvas.control_type)
            || self.verified_actions.len() > 16
            || (self.selected_canvas.is_some() && self.routes != AuthorityRoutes::Essential)
        {
            return Err(Error::Invalid);
        }
        canvas.canvas_rect.validate()?;
        let mut seen = 0u32;
        for action in &self.verified_actions {
            if !(1..=16).contains(action)
                || (self.routes == AuthorityRoutes::Essential && *action > 10)
                || seen & (1 << *action) != 0
            {
                return Err(Error::Invalid);
            }
            seen |= 1 << *action;
        }
        Ok(())
    }
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

#[cfg(test)]
mod authority_observation_tests {
    use super::*;
    fn authority(selected: bool) -> Authority {
        let canvas = CanvasProof {
            runtime_id_hash: "b".repeat(64),
            process_id: 7,
            sampled_qpc_100ns: 101,
            class_name: "KisOpenGLCanvas2".into(),
            control_type: 50026,
            canvas_rect: Rect {
                x: 0,
                y: 0,
                width: 800,
                height: 600,
            },
            profile_digest: "a".repeat(64),
        };
        Authority {
            identity: ImageIdentity {
                executable_name: "krita.exe".into(),
                executable_blake3: "c".repeat(64),
                executable_bytes: 1024,
                file_version: Some("5.3.4.0".into()),
                package_full_name: None,
                package_version: None,
            },
            profile_digest: "a".repeat(64),
            verified_actions: vec![1, 2, 3, 6, 7, 8, 9],
            routes: AuthorityRoutes::Essential,
            focus: (!selected).then(|| canvas.clone()),
            selected_canvas: selected.then_some(canvas),
        }
    }
    #[test]
    fn selected_canvas_round_trip_never_satisfies_initial_focus() -> Result<()> {
        for selected in [false, true] {
            let original = authority(selected);
            let bytes = serde_json::to_vec(&original).map_err(|_| Error::Invalid)?;
            let decoded: Authority = serde_json::from_slice(&bytes).map_err(|_| Error::Invalid)?;
            assert_eq!(decoded.canvas()?, original.canvas()?);
            assert_eq!(decoded.focused_canvas().is_ok(), !selected);
            assert_eq!(decoded.selected_canvas.is_some(), selected);
        }
        Ok(())
    }
    #[test]
    fn dual_missing_and_unknown_canvas_variants_refuse_at_deserialization() -> Result<()> {
        let original = serde_json::to_value(authority(false)).map_err(|_| Error::Invalid)?;
        let mut dual = original.clone();
        dual["selected_canvas"] = dual["focus"].clone();
        let mut missing = original.clone();
        missing["focus"] = serde_json::Value::Null;
        let mut missing_focus_field = original.clone();
        missing_focus_field
            .as_object_mut()
            .ok_or(Error::Invalid)?
            .remove("focus");
        let mut missing_selected_field = original.clone();
        missing_selected_field
            .as_object_mut()
            .ok_or(Error::Invalid)?
            .remove("selected_canvas");
        let mut unknown = original;
        unknown["guessed_focus"] = serde_json::Value::Bool(true);
        for value in [
            dual,
            missing,
            missing_focus_field,
            missing_selected_field,
            unknown,
        ] {
            assert!(serde_json::from_value::<Authority>(value).is_err());
        }
        Ok(())
    }
    #[test]
    fn selected_canvas_cannot_admit_legacy_routes_or_extended_key_actions() -> Result<()> {
        let mut selected = authority(true);
        selected.routes = AuthorityRoutes::Legacy;
        assert_eq!(selected.validate(), Err(Error::Invalid));
        selected.routes = AuthorityRoutes::Essential;
        selected.verified_actions.push(11);
        assert_eq!(selected.validate(), Err(Error::Invalid));
        let mut focused = authority(false);
        focused.verified_actions.push(11);
        assert_eq!(focused.validate(), Err(Error::Invalid));
        focused.routes = AuthorityRoutes::Legacy;
        focused.validate()?;
        Ok(())
    }
    #[test]
    fn canvas_digest_clock_geometry_and_duplicate_actions_cannot_be_authority() -> Result<()> {
        for mode in 0..5 {
            let mut selected = authority(true);
            let canvas = selected.selected_canvas.as_mut().ok_or(Error::Invalid)?;
            match mode {
                0 => canvas.profile_digest = "d".repeat(64),
                1 => canvas.sampled_qpc_100ns = 0,
                2 => canvas.canvas_rect.width = 0,
                3 => canvas.process_id = 0,
                _ => selected.verified_actions.push(1),
            }
            let bytes = serde_json::to_vec(&selected).map_err(|_| Error::Invalid)?;
            assert!(serde_json::from_slice::<Authority>(&bytes).is_err());
        }
        Ok(())
    }
    #[test]
    fn initial_and_refreshed_observations_must_match_the_selected_target() -> Result<()> {
        let rect = Rect {
            x: 0,
            y: 0,
            width: 800,
            height: 600,
        };
        let target = Target {
            token: "018bcfe5-6800-7000-8000-000000000002".into(),
            window: 1,
            process_id: 7,
            thread_id: 8,
            process_created: 9,
            window_rect: rect,
            frame_rect: rect,
            client_rect: rect,
            dpi: 96,
            integrity: 0,
        };
        for selected in [false, true] {
            let mut observation = authority(selected);
            observation.canvas_for_target(&target)?;
            let mut wrong_target = target.clone();
            wrong_target.process_id = 8;
            assert!(observation.canvas_for_target(&wrong_target).is_err());
            let canvas = if selected {
                &mut observation.selected_canvas
            } else {
                &mut observation.focus
            };
            canvas.as_mut().ok_or(Error::Invalid)?.canvas_rect.x = 1;
            assert!(observation.canvas_for_target(&target).is_err());
        }
        Ok(())
    }
}
