//! Exact measured toolbar routes. A valid shape never supplies runtime authority.
use super::{Profile, digest, text};
use crate::{Error, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Selector {
    pub class_name: String,
    pub automation_id: String,
    pub name: String,
    pub control_type: i32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Route {
    /// One unique direct child of the selected native root. Never desktop scope.
    pub anchor: Selector,
    /// One unique descendant within that specific anchor; no ordinal identity.
    pub leaf: Selector,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolbarAction {
    pub action: u32,
    pub route: Route,
    pub effect_receipt_digest: String,
    pub guard_receipt_digest: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LiveProfile {
    pub schema_version: u32,
    pub metadata: Profile,
    pub measured_tree_digest: String,
    pub watched_preset: String,
    pub watched_blending_mode: String,
    pub watched_preserve_alpha: bool,
    pub watched_tool_ids: Vec<String>,
    pub actions: Vec<ToolbarAction>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolSettings {
    pub freehand_selected: bool,
    pub selected_tool_id: String,
    pub eraser_mode: bool,
    pub preset: String,
    pub blending_mode: String,
    pub preserve_alpha: bool,
}
impl ToolSettings {
    /// Only settings relevant to these observed toolbar routes, not a complete
    /// Krita settings digest. Mode may toggle; it is observed separately.
    pub fn settings_bytes(&self) -> Vec<u8> {
        let mut out = b"M4-Krita-Watched-Settings-v1\0".to_vec();
        for value in [&self.preset, &self.blending_mode] {
            out.extend_from_slice(&(value.len() as u32).to_le_bytes());
            out.extend_from_slice(value.as_bytes());
        }
        out.push(u8::from(self.preserve_alpha));
        out
    }
}
fn selector(class: &str, id: &str, name: &str, control_type: i32) -> Selector {
    Selector {
        class_name: class.into(),
        automation_id: id.into(),
        name: name.into(),
        control_type,
    }
}
pub fn root_selector() -> Selector {
    // Name is deliberately not root identity; the title changes with history.
    selector("KisMainWindow", "MainWindow#1", "", 50032)
}
pub fn brushes_anchor() -> Selector {
    selector(
        "KisToolBar",
        "MainWindow#1.BrushesAndStuff",
        "Brushes and Stuff",
        50021,
    )
}
pub fn preset_route() -> Route {
    Route {
        anchor: selector("QStatusBar", "", "", 50017),
        leaf: selector("KSqueezedTextLabel", "statsBarStatusLabel", "", 50020),
    }
}
pub fn blend_route() -> Route {
    Route {
        anchor: brushes_anchor(),
        leaf: selector("KisCompositeOpComboBox", "", "", 50003),
    }
}
pub fn alpha_route() -> Route {
    Route {
        anchor: brushes_anchor(),
        leaf: selector("QToolButton", "", "Preserve Alpha", 50002),
    }
}
pub fn route_for(action: u32) -> Result<Route> {
    match action {
        1 | 2 => Ok(Route {
            anchor: selector("KisToolBar", "MainWindow#1.editToolBar", "Edit", 50021),
            leaf: selector(
                "QToolButton",
                "",
                if action == 1 { "Undo" } else { "Redo" },
                50000,
            ),
        }),
        3 => Ok(Route {
            anchor: selector("KoToolBoxDocker", "MainWindow#1.ToolBox", "Toolbox", 50032),
            leaf: selector(
                "KoToolBoxButton",
                "0 Krita/Shape.KritaShape/KisToolBrush",
                "",
                50002,
            ),
        }),
        // Explicit toggle. Selecting the freehand tool does not clear eraser mode.
        4 => Ok(Route {
            anchor: brushes_anchor(),
            leaf: selector("QToolButton", "", "Set eraser mode", 50002),
        }),
        _ => Err(Error::Unavailable),
    }
}
pub const BRUSH_ID: &str = "0 Krita/Shape.KritaShape/KisToolBrush";
pub const RECTANGLE_ID: &str = "0 Krita/Shape.KritaShape/KisToolRectangle";
/// Only a root-owned blank-editor fixture can request this transition. It is
/// absent from all production action IDs and profile route validation.
pub fn fixture_rectangle_route() -> Result<Route> {
    let mut route = route_for(3)?;
    route.leaf.automation_id = RECTANGLE_ID.into();
    Ok(route)
}
impl LiveProfile {
    pub fn decode(json: &str) -> Result<Self> {
        if json.len() > 32 * 1024 {
            return Err(Error::Limit);
        }
        let value: Self = serde_json::from_str(json).map_err(|_| Error::Invalid)?;
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<()> {
        self.metadata.validate()?;
        let m = &self.metadata;
        // A new version/provider needs new actual observations and source review.
        if self.schema_version != 2
            || !m.actions.is_empty()
            || m.executable_name != "krita.exe"
            || m.file_version.as_deref() != Some("5.3.4.0")
            || m.executable_blake3
                != "8e7e6999745d3f84cf402796717f86c05e39ba2398af0e3758888be333d50720"
            || m.executable_bytes != 273624
            || m.package_full_name.is_some()
            || m.package_version.is_some()
            || m.tool_id != "KisToolBrush"
            || !digest(&self.measured_tree_digest)
            || !text(&self.watched_preset, 256, false)
            || !text(&self.watched_blending_mode, 128, false)
            || self.watched_tool_ids.is_empty()
            || self.watched_tool_ids.len() > 2
            || self
                .watched_tool_ids
                .iter()
                .any(|id| !matches!(id.as_str(), BRUSH_ID | RECTANGLE_ID))
            || self
                .watched_tool_ids
                .iter()
                .enumerate()
                .any(|(n, id)| self.watched_tool_ids[..n].contains(id))
            || m.canvas_selector.framework_id != "Qt"
            || m.canvas_selector.class_name != "KisOpenGLCanvas2"
            || !m.canvas_selector.automation_id.is_empty()
            || m.canvas_selector.control_type != 50026
            || self.actions.is_empty()
            || self.actions.len() > 4
        {
            return Err(Error::Unavailable);
        }
        let mut seen = 0u32;
        for action in &self.actions {
            if action.route != route_for(action.action)?
                || seen & (1 << action.action) != 0
                || !digest(&action.effect_receipt_digest)
                || !digest(&action.guard_receipt_digest)
            {
                return Err(Error::Invalid);
            }
            seen |= 1 << action.action;
        }
        Ok(())
    }
    pub fn watch(&self, actual: &ToolSettings) -> Result<()> {
        if !self.watched_tool_ids.contains(&actual.selected_tool_id)
            || actual.freehand_selected != (actual.selected_tool_id == BRUSH_ID)
            || actual.preset != self.watched_preset
            || actual.blending_mode != self.watched_blending_mode
            || actual.preserve_alpha != self.watched_preserve_alpha
        {
            return Err(Error::Unavailable);
        }
        Ok(())
    }
    pub fn action(&self, action: u32) -> Result<&ToolbarAction> {
        self.actions
            .iter()
            .find(|v| v.action == action)
            .ok_or(Error::Unavailable)
    }
    pub fn permit(&self, action: u32, state: &ToolSettings) -> Result<()> {
        self.watch(state)?;
        self.action(action)?;
        if action == 4 && !state.freehand_selected {
            return Err(Error::Unavailable);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::CanvasSelector;
    fn fixture() -> Result<LiveProfile> {
        Ok(LiveProfile {
            schema_version: 2,
            metadata: Profile {
                schema_version: 1,
                acceptance_receipt_digest: "a".repeat(64),
                guard_lifecycle_receipt_digest: "b".repeat(64),
                executable_name: "krita.exe".into(),
                executable_blake3:
                    "8e7e6999745d3f84cf402796717f86c05e39ba2398af0e3758888be333d50720".into(),
                executable_bytes: 273624,
                file_version: Some("5.3.4.0".into()),
                package_full_name: None,
                package_version: None,
                tool_id: "KisToolBrush".into(),
                settings_digest: "c".repeat(64),
                canvas_selector: CanvasSelector {
                    framework_id: "Qt".into(),
                    class_name: "KisOpenGLCanvas2".into(),
                    automation_id: "".into(),
                    control_type: 50026,
                    require_keyboard_focus: true,
                },
                actions: vec![],
            },
            measured_tree_digest: "d".repeat(64),
            watched_preset: "b) Basic-5 Size Opacity".into(),
            watched_blending_mode: "Normal".into(),
            watched_preserve_alpha: false,
            watched_tool_ids: vec![BRUSH_ID.into()],
            actions: vec![ToolbarAction {
                action: 4,
                route: route_for(4)?,
                effect_receipt_digest: "e".repeat(64),
                guard_receipt_digest: "f".repeat(64),
            }],
        })
    }
    #[test]
    fn imported_schema_one_does_not_advertise_live_actions() -> Result<()> {
        let p = fixture()?;
        let json = serde_json::to_string(&p.metadata).map_err(|_| Error::Invalid)?;
        assert_eq!(LiveProfile::decode(&json), Err(Error::Invalid));
        Ok(())
    }
    #[test]
    fn numeric_size_and_guessed_keys_are_unavailable() -> Result<()> {
        assert_eq!(route_for(6), Err(Error::Unavailable));
        let mut p = fixture()?;
        p.actions[0].route.leaf.name.clear();
        assert_eq!(p.validate(), Err(Error::Invalid));
        Ok(())
    }
    #[test]
    fn duplicate_or_unbound_effect_routes_refuse() -> Result<()> {
        let mut p = fixture()?;
        p.actions.push(p.actions[0].clone());
        assert_eq!(p.validate(), Err(Error::Invalid));
        p.actions.pop();
        p.actions[0].effect_receipt_digest.clear();
        assert_eq!(p.validate(), Err(Error::Invalid));
        Ok(())
    }
    #[test]
    fn tool_or_watched_settings_change_refuses_but_known_toggle_transition_survives() -> Result<()>
    {
        let p = fixture()?;
        p.validate()?;
        let mut state = ToolSettings {
            freehand_selected: true,
            eraser_mode: false,
            selected_tool_id: BRUSH_ID.into(),
            preset: p.watched_preset.clone(),
            blending_mode: "Normal".into(),
            preserve_alpha: false,
        };
        p.watch(&state)?;
        let bytes = state.settings_bytes();
        state.eraser_mode = true;
        p.watch(&state)?;
        assert_eq!(state.settings_bytes(), bytes);
        state.freehand_selected = false;
        assert_eq!(p.watch(&state), Err(Error::Unavailable));
        state.freehand_selected = true;
        state.blending_mode = "Multiply".into();
        assert_eq!(p.watch(&state), Err(Error::Unavailable));
        Ok(())
    }
}
