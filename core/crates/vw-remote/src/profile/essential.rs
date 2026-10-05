//! Measured essential controls. Decoding describes a reviewed route; only a
//! trusted packaged digest and current native proof can supply action authority.
use super::{ImageIdentity, Profile, digest, live, text};
use crate::{Error, Result};
use serde::{Deserialize, Serialize};

pub const PAINT_PACKAGE: &str = "Microsoft.Paint_11.2605.81.0_x64__8wekyb3d8bbwe";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourcePolicy {
    Ordinary,
    InstalledPaint,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Editor {
    Krita,
    Paint,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NumericKey {
    Left,
    Right,
    Up,
    Down,
}
impl NumericKey {
    pub fn opposite(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
            Self::Up => Self::Down,
            Self::Down => Self::Up,
        }
    }
    pub fn virtual_key(self) -> u16 {
        match self {
            Self::Left => 0x25,
            Self::Right => 0x27,
            Self::Up => 0x26,
            Self::Down => 0x28,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RangeShape {
    pub minimum: f64,
    pub maximum: f64,
    pub small_change: f64,
    pub large_change: f64,
}
impl RangeShape {
    pub fn validate(&self) -> Result<()> {
        self.validate_step(true)
    }
    /// A wheel detent is measured causally, never computed from a provider step.
    /// Zero preserves an unknown/unadvertised increment; it grants no arrow key.
    pub fn validate_wheel(&self) -> Result<()> {
        self.validate_step(false)
    }
    fn validate_step(&self, require_positive_step: bool) -> Result<()> {
        if [
            self.minimum,
            self.maximum,
            self.small_change,
            self.large_change,
        ]
        .iter()
        .any(|value| !value.is_finite())
            || self.minimum >= self.maximum
            || self.minimum < -1_000_000.0
            || self.maximum > 1_000_000.0
            || self.small_change < 0.0
            || (require_positive_step && self.small_change == 0.0)
            || self.large_change < 0.0
            || self.small_change > self.maximum - self.minimum
            || self.large_change > self.maximum - self.minimum
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}

/// Anonymous controls need a unique current match inside this exact named
/// ancestor AND an actual calibration receipt. A range alone has no meaning.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NumericRoute {
    pub route: live::Route,
    pub range: RangeShape,
    pub key: NumericKey,
    pub semantic_receipt_digest: String,
    pub settings_receipt_digest: String,
}

/// Finite wheel over one actual rooted numeric element, never the canvas.
/// The exact direction/role requires causal, restoration and budget receipts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NumericWheelRoute {
    pub route: live::Route,
    pub range: RangeShape,
    pub wheel_delta: i32,
    pub semantic_receipt_digest: String,
    pub settings_receipt_digest: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaintToolbar {
    Undo,
    Redo,
    Brush,
    Eraser,
    ZoomIn,
    ZoomOut,
    FitCanvas,
}
impl PaintToolbar {
    pub fn action(self) -> u32 {
        match self {
            Self::Undo => 1,
            Self::Redo => 2,
            Self::Brush => 3,
            Self::Eraser => 5,
            Self::ZoomIn => 8,
            Self::ZoomOut => 9,
            Self::FitCanvas => 10,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PaintNumeric {
    pub range: RangeShape,
    pub key: NumericKey,
    pub semantic_receipt_digest: String,
    pub settings_receipt_digest: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ControlRoute {
    Toolbar { route: live::Route },
    FocusedNumeric { numeric: NumericRoute },
    ElementWheel { numeric: NumericWheelRoute },
    PaintToolbar { control: PaintToolbar },
    PaintSize { numeric: PaintNumeric },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EssentialAction {
    pub action: u32,
    pub route: ControlRoute,
    pub effect_receipt_digest: String,
    pub guard_receipt_digest: String,
    pub restoration_receipt_digest: String,
    pub production_budget_receipt_digest: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EssentialProfile {
    pub schema_version: u32,
    pub editor: Editor,
    pub source_policy: SourcePolicy,
    pub metadata: Profile,
    pub measured_tree_digest: String,
    pub watched_settings_digest: String,
    pub provider_budget_ms: u32,
    pub parent_exchange_budget_ms: u32,
    pub actions: Vec<EssentialAction>,
}

fn exact_identity(editor: Editor, identity: &ImageIdentity) -> bool {
    match editor {
        Editor::Krita => {
            identity.executable_name == "krita.exe"
                && identity.executable_blake3
                    == "8e7e6999745d3f84cf402796717f86c05e39ba2398af0e3758888be333d50720"
                && identity.executable_bytes == 273624
                && identity.file_version.as_deref() == Some("5.3.4.0")
                && identity.package_full_name.is_none()
                && identity.package_version.is_none()
        }
        Editor::Paint => {
            identity.executable_name == "mspaint.exe"
                && identity.executable_blake3
                    == "b2a7f30ffa5f4864fbe63b49c64153f1d75a278f5771f14e59ffb6be4f65792f"
                && identity.executable_bytes == 4_272_128
                && identity.file_version.as_deref() == Some("11.2605.81.0")
                && identity.package_full_name.as_deref() == Some(PAINT_PACKAGE)
                && identity.package_version.as_deref() == Some("11.2605.81.0")
        }
    }
}
fn selector_shape(selector: &live::Selector) -> bool {
    text(&selector.class_name, 256, false)
        && text(&selector.automation_id, 256, true)
        && text(&selector.name, 256, true)
        && (50000..=50100).contains(&selector.control_type)
}
fn numeric_shape(editor: Editor, action: u32, numeric: &NumericRoute) -> Result<()> {
    numeric.range.validate()?;
    numeric_selector_shape(editor, action, numeric)
}
fn numeric_selector_shape(editor: Editor, action: u32, numeric: &NumericRoute) -> Result<()> {
    if !digest(&numeric.semantic_receipt_digest)
        || !digest(&numeric.settings_receipt_digest)
        || !selector_shape(&numeric.route.anchor)
        || !selector_shape(&numeric.route.leaf)
        || !matches!(action, 6..=9)
    {
        return Err(Error::Invalid);
    }
    // Route syntax is admitted separately from the observed semantic/effect
    // receipt. No ordinal, absolute coordinate or imported key chord is present.
    match editor {
        Editor::Krita
            if matches!(action, 6 | 7)
                && numeric.route.anchor == live::brushes_anchor()
                && numeric.route.leaf.class_name == "KisDoubleSliderSpinBox"
                && numeric.route.leaf.automation_id.is_empty()
                && numeric.route.leaf.name.is_empty()
                && numeric.route.leaf.control_type == 50016 =>
        {
            Ok(())
        }
        Editor::Krita
            if matches!(action, 8 | 9)
                && numeric.route.anchor.class_name == "QStatusBar"
                && numeric.route.anchor.automation_id.is_empty()
                && numeric.route.anchor.name.is_empty()
                && numeric.route.anchor.control_type == 50017
                && numeric.route.leaf.class_name == "QSlider"
                && numeric.route.leaf.automation_id.is_empty()
                && numeric.route.leaf.name.is_empty()
                && numeric.route.leaf.control_type == 50015 =>
        {
            Ok(())
        }
        // Native Paint lookup uses the exact XAML pane + direct Size/Zoom
        // slider/Thumb path; this Qt Route form cannot grant a Paint numeric.
        Editor::Paint => Err(Error::Unavailable),
        _ => Err(Error::Unavailable),
    }
}
impl EssentialProfile {
    pub fn decode(json: &str) -> Result<Self> {
        if json.len() > 32 * 1024 {
            return Err(Error::Limit);
        }
        let profile: Self = serde_json::from_str(json).map_err(|_| Error::Invalid)?;
        profile.validate()?;
        Ok(profile)
    }
    pub fn validate(&self) -> Result<()> {
        self.metadata.validate()?;
        if self.schema_version != 3
            || !self.metadata.actions.is_empty()
            || !exact_identity(self.editor, &self.metadata.identity())
            || !matches!(
                (self.editor, self.source_policy),
                (Editor::Krita, SourcePolicy::Ordinary)
                    | (Editor::Paint, SourcePolicy::InstalledPaint)
            )
            || !digest(&self.measured_tree_digest)
            || !digest(&self.watched_settings_digest)
            || self.watched_settings_digest != self.metadata.settings_digest
            || self.provider_budget_ms != 180
            || self.parent_exchange_budget_ms != 250
            || self.actions.is_empty()
            || self.actions.len() > 10
        {
            return Err(Error::Unavailable);
        }
        let canvas = &self.metadata.canvas_selector;
        if match self.editor {
            Editor::Krita => {
                canvas.framework_id != "Qt"
                    || canvas.class_name != "KisOpenGLCanvas2"
                    || !canvas.automation_id.is_empty()
                    || canvas.control_type != 50026
            }
            Editor::Paint => {
                canvas.framework_id != "XAML"
                    || canvas.class_name != "NamedContainerAutomationPeer"
                    || canvas.automation_id != "image"
                    || canvas.control_type != 50026
            }
        } {
            return Err(Error::Unavailable);
        }
        let mut seen = 0u32;
        for action in &self.actions {
            if !(1..=10).contains(&action.action)
                || seen & (1 << action.action) != 0
                || [
                    &action.effect_receipt_digest,
                    &action.guard_receipt_digest,
                    &action.restoration_receipt_digest,
                    &action.production_budget_receipt_digest,
                ]
                .iter()
                .any(|receipt| !digest(receipt))
            {
                return Err(Error::Invalid);
            }
            match &action.route {
                ControlRoute::Toolbar { route } => {
                    // Existing measured Krita tool/history selectors only;
                    // Paint named routes are handled in its separate schema.
                    if self.editor != Editor::Krita || *route != live::route_for(action.action)? {
                        return Err(Error::Unavailable);
                    }
                }
                ControlRoute::FocusedNumeric { numeric } => {
                    numeric_shape(self.editor, action.action, numeric)?
                }
                ControlRoute::ElementWheel { numeric } => {
                    if !matches!(numeric.wheel_delta, -120 | 120) {
                        return Err(Error::Invalid);
                    }
                    // Existing root/shape/semantic receipt validation; the key
                    // is unused for this route and cannot become emitted input.
                    numeric.range.validate_wheel()?;
                    numeric_selector_shape(
                        self.editor,
                        action.action,
                        &NumericRoute {
                            route: numeric.route.clone(),
                            range: numeric.range.clone(),
                            key: NumericKey::Up,
                            semantic_receipt_digest: numeric.semantic_receipt_digest.clone(),
                            settings_receipt_digest: numeric.settings_receipt_digest.clone(),
                        },
                    )?;
                }
                ControlRoute::PaintToolbar { control } => {
                    if self.editor != Editor::Paint || action.action != control.action() {
                        return Err(Error::Unavailable);
                    }
                }
                ControlRoute::PaintSize { numeric } => {
                    numeric.range.validate()?;
                    if self.editor != Editor::Paint
                        || !matches!(action.action, 6 | 7)
                        || !digest(&numeric.semantic_receipt_digest)
                        || !digest(&numeric.settings_receipt_digest)
                    {
                        return Err(Error::Unavailable);
                    }
                }
            }
            seen |= 1 << action.action;
        }
        // Increasing/decreasing controls must address the same measured widget,
        // semantic/settings receipt and shape, with opposite paired keys. This
        // supplies no range-to-pixel meaning; root still admits causal receipts.
        for (decrease, increase) in [(6, 7), (9, 8)] {
            if let (Ok(a), Ok(b)) = (self.action(decrease), self.action(increase)) {
                match (&a.route, &b.route) {
                    (
                        ControlRoute::FocusedNumeric { numeric: a },
                        ControlRoute::FocusedNumeric { numeric: b },
                    ) => {
                        if a.route != b.route
                            || a.range != b.range
                            || a.semantic_receipt_digest != b.semantic_receipt_digest
                            || a.settings_receipt_digest != b.settings_receipt_digest
                            || a.key.opposite() != b.key
                        {
                            return Err(Error::Invalid);
                        }
                    }
                    (
                        ControlRoute::ElementWheel { numeric: a },
                        ControlRoute::ElementWheel { numeric: b },
                    ) => {
                        if a.route != b.route
                            || a.range != b.range
                            || a.semantic_receipt_digest != b.semantic_receipt_digest
                            || a.settings_receipt_digest != b.settings_receipt_digest
                            || a.wheel_delta != -b.wheel_delta
                        {
                            return Err(Error::Invalid);
                        }
                    }
                    (
                        ControlRoute::PaintSize { numeric: a },
                        ControlRoute::PaintSize { numeric: b },
                    ) => {
                        if a.range != b.range
                            || a.semantic_receipt_digest != b.semantic_receipt_digest
                            || a.settings_receipt_digest != b.settings_receipt_digest
                            || a.key.opposite() != b.key
                        {
                            return Err(Error::Invalid);
                        }
                    }
                    (
                        ControlRoute::PaintToolbar {
                            control: PaintToolbar::ZoomOut,
                        },
                        ControlRoute::PaintToolbar {
                            control: PaintToolbar::ZoomIn,
                        },
                    ) => {}
                    _ => return Err(Error::Invalid),
                }
            }
        }
        Ok(())
    }
    pub fn action(&self, action: u32) -> Result<&EssentialAction> {
        self.actions
            .iter()
            .find(|entry| entry.action == action)
            .ok_or(Error::Unavailable)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    #[test]
    fn anonymous_numeric_without_semantic_settings_receipts_refuses() -> Result<()> {
        let mut route = NumericRoute {
            route: live::Route {
                anchor: live::brushes_anchor(),
                leaf: live::Selector {
                    class_name: "KisDoubleSliderSpinBox".into(),
                    automation_id: String::new(),
                    name: String::new(),
                    control_type: 50016,
                },
            },
            range: RangeShape {
                minimum: 0.01,
                maximum: 1000.0,
                small_change: 1.0,
                large_change: 10.0,
            },
            key: NumericKey::Up,
            semantic_receipt_digest: "a".repeat(64),
            settings_receipt_digest: "b".repeat(64),
        };
        numeric_shape(Editor::Krita, 7, &route)?;
        route.semantic_receipt_digest.clear();
        assert_eq!(numeric_shape(Editor::Krita, 7, &route), Err(Error::Invalid));
        route.semantic_receipt_digest = "a".repeat(64);
        route.range.maximum = f64::NAN;
        assert_eq!(numeric_shape(Editor::Krita, 7, &route), Err(Error::Invalid));
        Ok(())
    }
    #[test]
    fn observed_package_and_version_must_match_exact_editor_policy() {
        let identity = ImageIdentity {
            executable_name: "mspaint.exe".into(),
            executable_blake3: "b2a7f30ffa5f4864fbe63b49c64153f1d75a278f5771f14e59ffb6be4f65792f"
                .into(),
            executable_bytes: 4_272_128,
            file_version: Some("11.2605.81.0".into()),
            package_full_name: Some(PAINT_PACKAGE.into()),
            package_version: Some("11.2605.81.0".into()),
        };
        assert!(exact_identity(Editor::Paint, &identity));
        assert!(!exact_identity(Editor::Krita, &identity));
        let mut changed = identity;
        changed.package_version = Some("11.2605.82.0".into());
        assert!(!exact_identity(Editor::Paint, &changed));
    }
    pub(crate) fn fixture(editor: Editor) -> Result<EssentialProfile> {
        let paint = editor == Editor::Paint;
        let identity = if paint {
            ImageIdentity {
                executable_name: "mspaint.exe".into(),
                executable_blake3:
                    "b2a7f30ffa5f4864fbe63b49c64153f1d75a278f5771f14e59ffb6be4f65792f".into(),
                executable_bytes: 4_272_128,
                file_version: Some("11.2605.81.0".into()),
                package_full_name: Some(PAINT_PACKAGE.into()),
                package_version: Some("11.2605.81.0".into()),
            }
        } else {
            ImageIdentity {
                executable_name: "krita.exe".into(),
                executable_blake3:
                    "8e7e6999745d3f84cf402796717f86c05e39ba2398af0e3758888be333d50720".into(),
                executable_bytes: 273624,
                file_version: Some("5.3.4.0".into()),
                package_full_name: None,
                package_version: None,
            }
        };
        Ok(EssentialProfile {
            schema_version: 3,
            editor,
            source_policy: if paint {
                SourcePolicy::InstalledPaint
            } else {
                SourcePolicy::Ordinary
            },
            metadata: Profile {
                schema_version: 1,
                acceptance_receipt_digest: "a".repeat(64),
                guard_lifecycle_receipt_digest: "b".repeat(64),
                executable_name: identity.executable_name,
                executable_blake3: identity.executable_blake3,
                executable_bytes: identity.executable_bytes,
                file_version: identity.file_version,
                package_full_name: identity.package_full_name,
                package_version: identity.package_version,
                tool_id: if paint {
                    "Paint.Brush.Eraser"
                } else {
                    "KisToolBrush"
                }
                .into(),
                settings_digest: "c".repeat(64),
                canvas_selector: super::super::CanvasSelector {
                    framework_id: if paint { "XAML" } else { "Qt" }.into(),
                    class_name: if paint {
                        "NamedContainerAutomationPeer"
                    } else {
                        "KisOpenGLCanvas2"
                    }
                    .into(),
                    automation_id: if paint { "image" } else { "" }.into(),
                    control_type: 50026,
                    require_keyboard_focus: true,
                },
                actions: vec![],
            },
            measured_tree_digest: "d".repeat(64),
            watched_settings_digest: "c".repeat(64),
            provider_budget_ms: 180,
            parent_exchange_budget_ms: 250,
            actions: vec![EssentialAction {
                action: 3,
                route: if paint {
                    ControlRoute::PaintToolbar {
                        control: PaintToolbar::Brush,
                    }
                } else {
                    ControlRoute::Toolbar {
                        route: live::route_for(3)?,
                    }
                },
                effect_receipt_digest: "e".repeat(64),
                guard_receipt_digest: "f".repeat(64),
                restoration_receipt_digest: "1".repeat(64),
                production_budget_receipt_digest: "2".repeat(64),
            }],
        })
    }
    #[test]
    fn numeric_range_nonfinite_out_of_bounds_and_nonpositive_steps_refuse() {
        let valid = RangeShape {
            minimum: 0.0,
            maximum: 90.0,
            small_change: 1.0,
            large_change: 10.0,
        };
        for bad in [
            RangeShape {
                minimum: f64::NEG_INFINITY,
                ..valid.clone()
            },
            RangeShape {
                maximum: f64::NAN,
                ..valid.clone()
            },
            RangeShape {
                small_change: 0.0,
                ..valid.clone()
            },
            RangeShape {
                large_change: 91.0,
                ..valid.clone()
            },
            RangeShape {
                minimum: 90.0,
                ..valid.clone()
            },
            RangeShape {
                maximum: 1_000_001.0,
                ..valid.clone()
            },
        ] {
            assert_eq!(bad.validate(), Err(Error::Invalid));
        }
    }
    #[test]
    fn essential_actions_need_all_four_causal_guard_restore_and_budget_receipts() -> Result<()> {
        let p = fixture(Editor::Paint)?;
        p.validate()?;
        for field in 0..4 {
            let mut bad = p.clone();
            let action = &mut bad.actions[0];
            match field {
                0 => action.effect_receipt_digest.clear(),
                1 => action.guard_receipt_digest.clear(),
                2 => action.restoration_receipt_digest.clear(),
                _ => action.production_budget_receipt_digest.clear(),
            }
            assert_eq!(bad.validate(), Err(Error::Invalid));
        }
        Ok(())
    }
    #[test]
    fn larger_budgets_or_untrusted_keyboard_metadata_cannot_grant_routes() -> Result<()> {
        let mut p = fixture(Editor::Krita)?;
        p.provider_budget_ms = 181;
        assert_eq!(p.validate(), Err(Error::Unavailable));
        p.provider_budget_ms = 180;
        p.parent_exchange_budget_ms = 251;
        assert_eq!(p.validate(), Err(Error::Unavailable));
        p.parent_exchange_budget_ms = 250;
        p.metadata.actions.push(super::super::Action {
            action: 3,
            keys: vec![0x42],
            native_batch_digest: "a".repeat(64),
            effect_receipt_digest: "b".repeat(64),
        });
        assert_eq!(p.validate(), Err(Error::Unavailable));
        Ok(())
    }
    #[test]
    fn wrong_editor_route_duplicate_action_and_unknown_action_refuse() -> Result<()> {
        let p = fixture(Editor::Paint)?;
        let mut bad = p.clone();
        bad.actions[0].route = ControlRoute::Toolbar {
            route: live::route_for(3)?,
        };
        assert_eq!(bad.validate(), Err(Error::Unavailable));
        bad = p.clone();
        bad.actions.push(bad.actions[0].clone());
        assert_eq!(bad.validate(), Err(Error::Invalid));
        bad = p;
        bad.actions[0].action = 16;
        assert_eq!(bad.validate(), Err(Error::Invalid));
        Ok(())
    }
    #[test]
    fn opposed_size_actions_require_same_semantic_widget_receipts_and_opposite_key() -> Result<()> {
        let mut p = fixture(Editor::Paint)?;
        let numeric = PaintNumeric {
            range: RangeShape {
                minimum: 0.0,
                maximum: 90.0,
                small_change: 1.0,
                large_change: 10.0,
            },
            key: NumericKey::Down,
            semantic_receipt_digest: "3".repeat(64),
            settings_receipt_digest: "4".repeat(64),
        };
        let mut smaller = p.actions[0].clone();
        smaller.action = 6;
        smaller.route = ControlRoute::PaintSize {
            numeric: numeric.clone(),
        };
        let mut larger = smaller.clone();
        larger.action = 7;
        larger.route = ControlRoute::PaintSize {
            numeric: PaintNumeric {
                key: NumericKey::Up,
                ..numeric
            },
        };
        p.actions = vec![smaller, larger];
        p.validate()?;
        if let ControlRoute::PaintSize { numeric } = &mut p.actions[1].route {
            numeric.key = NumericKey::Down;
        }
        assert_eq!(p.validate(), Err(Error::Invalid));
        if let ControlRoute::PaintSize { numeric } = &mut p.actions[1].route {
            numeric.key = NumericKey::Up;
            numeric.semantic_receipt_digest = "5".repeat(64);
        }
        assert_eq!(p.validate(), Err(Error::Invalid));
        Ok(())
    }
    #[test]
    fn closed_json_rejects_guessed_wheel_ordinal_coordinates_and_duplicate_schema() -> Result<()> {
        let p = fixture(Editor::Krita)?;
        let json = serde_json::to_string(&p).map_err(|_| Error::Invalid)?;
        assert!(
            EssentialProfile::decode(&json.replace(
                "\"schemaVersion\":3",
                "\"schemaVersion\":3,\"schemaVersion\":3"
            ))
            .is_err()
        );
        for json in [
            r#"{"kind":"canvas_wheel","wheel_delta":120}"#,
            r#"{"kind":"focused_numeric","ordinal":1}"#,
            r#"{"kind":"toolbar","point":[1,2]}"#,
        ] {
            assert!(serde_json::from_str::<ControlRoute>(json).is_err());
        }
        Ok(())
    }
    #[test]
    fn finite_element_wheel_requires_exact_calibration_and_opposed_route() -> Result<()> {
        let mut profile = fixture(Editor::Krita)?;
        let numeric = NumericWheelRoute {
            route: live::Route {
                anchor: live::brushes_anchor(),
                leaf: live::Selector {
                    class_name: "KisDoubleSliderSpinBox".into(),
                    automation_id: String::new(),
                    name: String::new(),
                    control_type: 50016,
                },
            },
            range: RangeShape {
                minimum: 0.01,
                maximum: 1000.0,
                small_change: 1.0,
                large_change: 10.0,
            },
            wheel_delta: -120,
            semantic_receipt_digest: "a".repeat(64),
            settings_receipt_digest: "b".repeat(64),
        };
        let mut smaller = profile.actions[0].clone();
        smaller.action = 6;
        smaller.route = ControlRoute::ElementWheel {
            numeric: numeric.clone(),
        };
        let mut larger = smaller.clone();
        larger.action = 7;
        larger.route = ControlRoute::ElementWheel {
            numeric: NumericWheelRoute {
                wheel_delta: 120,
                ..numeric
            },
        };
        profile.actions = vec![smaller, larger];
        profile.validate()?;
        if let ControlRoute::ElementWheel { numeric } = &mut profile.actions[1].route {
            numeric.wheel_delta = -120
        }
        assert_eq!(profile.validate(), Err(Error::Invalid));
        if let ControlRoute::ElementWheel { numeric } = &mut profile.actions[1].route {
            numeric.wheel_delta = 120;
            numeric.semantic_receipt_digest.clear()
        }
        assert_eq!(profile.validate(), Err(Error::Invalid));
        Ok(())
    }
    #[test]
    fn element_wheel_never_accepts_canvas_route_or_unbounded_delta() -> Result<()> {
        let mut profile = fixture(Editor::Krita)?;
        let mut action = profile.actions[0].clone();
        action.action = 8;
        action.route = ControlRoute::ElementWheel {
            numeric: NumericWheelRoute {
                route: live::Route {
                    anchor: live::Selector {
                        class_name: "QStatusBar".into(),
                        automation_id: String::new(),
                        name: String::new(),
                        control_type: 50017,
                    },
                    leaf: live::Selector {
                        class_name: "QSlider".into(),
                        automation_id: String::new(),
                        name: String::new(),
                        control_type: 50015,
                    },
                },
                range: RangeShape {
                    minimum: 0.0,
                    maximum: 18.0,
                    small_change: 1.0,
                    large_change: 1.0,
                },
                wheel_delta: 120,
                semantic_receipt_digest: "a".repeat(64),
                settings_receipt_digest: "b".repeat(64),
            },
        };
        profile.actions = vec![action];
        profile.validate()?;
        if let ControlRoute::ElementWheel { numeric } = &mut profile.actions[0].route {
            numeric.wheel_delta = 240
        }
        assert_eq!(profile.validate(), Err(Error::Invalid));
        if let ControlRoute::ElementWheel { numeric } = &mut profile.actions[0].route {
            numeric.wheel_delta = 120;
            numeric.route.leaf.class_name = "KisOpenGLCanvas2".into()
        }
        assert_eq!(profile.validate(), Err(Error::Unavailable));
        Ok(())
    }
    #[test]
    fn wheel_unknown_step_keeps_finite_bounds_and_arrow_step_requirements() -> Result<()> {
        let unknown = RangeShape {
            minimum: 0.01,
            maximum: 1000.0,
            small_change: 0.0,
            large_change: 0.0,
        };
        unknown.validate_wheel()?;
        assert_eq!(unknown.validate(), Err(Error::Invalid));
        for bad in [
            RangeShape {
                small_change: -1.0,
                ..unknown.clone()
            },
            RangeShape {
                small_change: f64::NAN,
                ..unknown.clone()
            },
            RangeShape {
                maximum: f64::INFINITY,
                ..unknown.clone()
            },
            RangeShape {
                large_change: -1.0,
                ..unknown.clone()
            },
            RangeShape {
                minimum: 1000.0,
                ..unknown.clone()
            },
            RangeShape {
                small_change: 1000.0,
                ..unknown.clone()
            },
        ] {
            assert_eq!(bad.validate_wheel(), Err(Error::Invalid));
        }
        Ok(())
    }
    #[test]
    fn zero_step_wheel_receipt_does_not_authorize_focused_or_paint_arrows() -> Result<()> {
        let mut p = fixture(Editor::Krita)?;
        let route = live::Route {
            anchor: live::brushes_anchor(),
            leaf: live::Selector {
                class_name: "KisDoubleSliderSpinBox".into(),
                automation_id: String::new(),
                name: String::new(),
                control_type: 50016,
            },
        };
        let range = RangeShape {
            minimum: 0.01,
            maximum: 1000.0,
            small_change: 0.0,
            large_change: 0.0,
        };
        p.actions[0].action = 7;
        p.actions[0].route = ControlRoute::ElementWheel {
            numeric: NumericWheelRoute {
                route: route.clone(),
                range: range.clone(),
                wheel_delta: 120,
                semantic_receipt_digest: "a".repeat(64),
                settings_receipt_digest: "b".repeat(64),
            },
        };
        p.validate()?;
        let admitted = p.clone();
        p.actions[0].effect_receipt_digest.clear();
        assert_eq!(p.validate(), Err(Error::Invalid));
        p = admitted;
        p.actions[0].route = ControlRoute::FocusedNumeric {
            numeric: NumericRoute {
                route,
                range: range.clone(),
                key: NumericKey::Up,
                semantic_receipt_digest: "a".repeat(64),
                settings_receipt_digest: "b".repeat(64),
            },
        };
        assert_eq!(p.validate(), Err(Error::Invalid));
        let mut paint = fixture(Editor::Paint)?;
        paint.actions[0].action = 7;
        paint.actions[0].route = ControlRoute::PaintSize {
            numeric: PaintNumeric {
                range,
                key: NumericKey::Up,
                semantic_receipt_digest: "a".repeat(64),
                settings_receipt_digest: "b".repeat(64),
            },
        };
        assert_eq!(paint.validate(), Err(Error::Invalid));
        Ok(())
    }
}
