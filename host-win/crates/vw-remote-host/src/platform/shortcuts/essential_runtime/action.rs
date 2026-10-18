//! Evidence-free execution syntax. Conversion is downstream of catalog admission;
//! calibration may measure syntax but can never manufacture admission receipts.
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use vw_remote::profile::{
    essential::{self, Editor, NumericKey, PaintToolbar, RangeShape},
    live,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Numeric {
    pub route: live::Route,
    pub range: RangeShape,
    pub key: NumericKey,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Wheel {
    pub route: live::Route,
    pub range: RangeShape,
    pub wheel_delta: i32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PaintNumeric {
    pub range: RangeShape,
    pub key: NumericKey,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum RouteSpec {
    Toolbar { route: live::Route },
    FocusedNumeric { numeric: Numeric },
    ElementWheel { numeric: Wheel },
    PaintToolbar { control: PaintToolbar },
    PaintSize { numeric: PaintNumeric },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ActionSpec {
    pub action: u32,
    pub route: RouteSpec,
}
impl From<&essential::EssentialAction> for ActionSpec {
    fn from(action: &essential::EssentialAction) -> Self {
        use essential::ControlRoute as R;
        let route = match &action.route {
            R::Toolbar { route } => RouteSpec::Toolbar {
                route: route.clone(),
            },
            R::FocusedNumeric { numeric: n } => RouteSpec::FocusedNumeric {
                numeric: Numeric {
                    route: n.route.clone(),
                    range: n.range.clone(),
                    key: n.key,
                },
            },
            R::ElementWheel { numeric: n } => RouteSpec::ElementWheel {
                numeric: Wheel {
                    route: n.route.clone(),
                    range: n.range.clone(),
                    wheel_delta: n.wheel_delta,
                },
            },
            R::PaintToolbar { control } => RouteSpec::PaintToolbar { control: *control },
            R::PaintSize { numeric: n } => RouteSpec::PaintSize {
                numeric: PaintNumeric {
                    range: n.range.clone(),
                    key: n.key,
                },
            },
        };
        Self {
            action: action.action,
            route,
        }
    }
}
fn numeric_route(editor: Editor, action: u32, route: &live::Route) -> Result<()> {
    let leaf = &route.leaf;
    let anchor = &route.anchor;
    let size = matches!(action, 6 | 7)
        && *anchor == live::brushes_anchor()
        && leaf.class_name == "KisDoubleSliderSpinBox"
        && leaf.control_type == 50016;
    let zoom = matches!(action, 8 | 9)
        && anchor.class_name == "QStatusBar"
        && anchor.automation_id.is_empty()
        && anchor.name.is_empty()
        && anchor.control_type == 50017
        && leaf.class_name == "QSlider"
        && leaf.control_type == 50015;
    if editor != Editor::Krita
        || !(size || zoom)
        || !leaf.automation_id.is_empty()
        || !leaf.name.is_empty()
    {
        return Err(Error::Unavailable);
    }
    Ok(())
}
impl ActionSpec {
    /// Closed syntax mirrors admitted production routes. This validates no role,
    /// effect, direction, restoration or catalog authority.
    pub(crate) fn validate_candidate(&self, editor: Editor) -> Result<()> {
        match &self.route {
            RouteSpec::Toolbar { route }
                if editor == Editor::Krita && *route == live::route_for(self.action)? =>
            {
                Ok(())
            }
            RouteSpec::FocusedNumeric { numeric } => {
                numeric.range.validate()?;
                numeric_route(editor, self.action, &numeric.route)
            }
            RouteSpec::ElementWheel { numeric } => {
                numeric.range.validate_wheel()?;
                if !matches!(numeric.wheel_delta, -120 | 120) {
                    return Err(Error::Invalid);
                }
                numeric_route(editor, self.action, &numeric.route)
            }
            RouteSpec::PaintToolbar { control }
                if editor == Editor::Paint && self.action == control.action() =>
            {
                Ok(())
            }
            RouteSpec::PaintSize { numeric }
                if editor == Editor::Paint && matches!(self.action, 6 | 7) =>
            {
                numeric.range.validate()
            }
            _ => Err(Error::Unavailable),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn calibration_syntax_accepts_no_evidence_fields_or_unsupported_action() -> Result<()> {
        let value = ActionSpec {
            action: 3,
            route: RouteSpec::PaintToolbar {
                control: PaintToolbar::Brush,
            },
        };
        value.validate_candidate(Editor::Paint)?;
        assert_eq!(
            value.validate_candidate(Editor::Krita),
            Err(Error::Unavailable)
        );
        let mut json = serde_json::to_value(value).map_err(|_| Error::Invalid)?;
        json["effectReceiptDigest"] = serde_json::json!("a".repeat(64));
        assert!(serde_json::from_value::<ActionSpec>(json).is_err());
        assert_eq!(
            ActionSpec {
                action: 10,
                route: RouteSpec::Toolbar {
                    route: live::route_for(3)?
                }
            }
            .validate_candidate(Editor::Krita),
            Err(Error::Unavailable)
        );
        Ok(())
    }
    #[test]
    fn zero_step_wheel_candidate_cannot_become_an_arrow_route() -> Result<()> {
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
        ActionSpec {
            action: 6,
            route: RouteSpec::ElementWheel {
                numeric: Wheel {
                    route: route.clone(),
                    range: range.clone(),
                    wheel_delta: -120,
                },
            },
        }
        .validate_candidate(Editor::Krita)?;
        assert_eq!(
            ActionSpec {
                action: 6,
                route: RouteSpec::FocusedNumeric {
                    numeric: Numeric {
                        route,
                        range,
                        key: NumericKey::Down
                    }
                }
            }
            .validate_candidate(Editor::Krita),
            Err(Error::Invalid)
        );
        Ok(())
    }
}
