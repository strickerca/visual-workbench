//! Restore only known observed tool/toggle state through the same guarded route.
use crate::{Error, Result, editor_probe::Source, platform};
use vw_remote::{
    Target,
    profile::live::{self, ToolSettings},
};
#[derive(serde::Serialize)]
pub(super) struct Restoration {
    pub status: &'static str,
    pub injections: Vec<platform::ClickMeasurement>,
    pub final_settings: Option<ToolSettings>,
    pub error: Option<Error>,
}
impl Restoration {
    pub fn inactive() -> Self {
        Self {
            status: "not_requested",
            injections: vec![],
            final_settings: None,
            error: None,
        }
    }
    pub fn failed(error: Error) -> Self {
        Self {
            status: "failed",
            injections: vec![],
            final_settings: None,
            error: Some(error),
        }
    }
}
pub(super) fn restore(
    source: &Source,
    target: &Target,
    owner: u32,
    baseline: &ToolSettings,
) -> Restoration {
    let mut injections = Vec::new();
    let result = (|| -> Result<ToolSettings> {
        source.verify(target, owner)?;
        let mut current = platform::measure(target, owner, 3)?;
        if current.settings.selected_tool_id != baseline.selected_tool_id {
            let action = match baseline.selected_tool_id.as_str() {
                live::BRUSH_ID => 3,
                live::RECTANGLE_ID => 90,
                _ => return Err(Error::Unavailable),
            };
            current = platform::measure(target, owner, action)?;
            source.verify(target, owner)?;
            let outcome = platform::measure_click(target, owner, action, &current)?;
            let error = outcome.error.clone();
            injections.push(outcome);
            if let Some(error) = error {
                return Err(error);
            }
        }
        source.verify(target, owner)?;
        current = platform::measure(target, owner, 4)?;
        if current.settings.eraser_mode != baseline.eraser_mode {
            source.verify(target, owner)?;
            let outcome = platform::measure_click(target, owner, 4, &current)?;
            let error = outcome.error.clone();
            injections.push(outcome);
            if let Some(error) = error {
                return Err(error);
            }
        }
        source.verify(target, owner)?;
        let settings = platform::measure(target, owner, 3)?.settings;
        source.verify(target, owner)?;
        // Unsupported preset/range/blend changes remain an explicit failure;
        // never reset preferences or infer an unnamed slider restoration.
        if &settings != baseline {
            return Err(Error::TargetChanged);
        }
        Ok(settings)
    })();
    match result {
        Ok(settings) => Restoration {
            status: "restored",
            injections,
            final_settings: Some(settings),
            error: None,
        },
        Err(error) => Restoration {
            status: "failed",
            injections,
            final_settings: None,
            error: Some(error),
        },
    }
}
