//! Trusted-package catalog syntax. Selection requires OS package facts from the
//! retained current target process; neither a title nor a failed file open selects
//! the installed-package policy. Every selected entry still needs SourceLease.
use super::{
    ImageIdentity, Profile,
    essential::{EssentialProfile, PAINT_PACKAGE, SourcePolicy},
};
use crate::{Error, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Entry {
    Essential { profile: EssentialProfile },
}
impl Entry {
    pub fn metadata(&self) -> &Profile {
        match self {
            Self::Essential { profile } => &profile.metadata,
        }
    }
    pub fn identity(&self) -> ImageIdentity {
        self.metadata().identity()
    }
    pub fn policy(&self) -> SourcePolicy {
        match self {
            Self::Essential { profile } => profile.source_policy,
        }
    }
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Essential { profile } => profile.validate(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Catalog {
    pub schema_version: u32,
    pub entries: Vec<Entry>,
}
impl Catalog {
    pub fn decode(json: &str) -> Result<Self> {
        if json.len() > 32 * 1024 {
            return Err(Error::Limit);
        }
        let catalog: Self = serde_json::from_str(json).map_err(|_| Error::Invalid)?;
        catalog.validate()?;
        Ok(catalog)
    }
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 4 || self.entries.is_empty() || self.entries.len() > 2 {
            return Err(Error::Invalid);
        }
        for (index, entry) in self.entries.iter().enumerate() {
            entry.validate()?;
            // One explicit policy/identity per OS package selection, avoiding an
            // arbitrary choice between overlapping action profiles.
            if self.entries[..index]
                .iter()
                .any(|prior| prior.policy() == entry.policy())
            {
                return Err(Error::Invalid);
            }
        }
        Ok(())
    }
    /// This parameter is a bounded actual GetPackageFullName observation on the
    /// retained selected process. It is not supplied by UI/client profile JSON.
    /// None means the API returned APPMODEL_ERROR_NO_PACKAGE, never query error.
    pub fn for_running_package(&self, package: Option<&str>) -> Result<&Entry> {
        self.applicable_to_running_package(package)?
            .ok_or(Error::Unavailable)
    }
    /// A valid catalog may have no applicable entry. This grants no shortcut
    /// authority and does not veto independently guarded selected-target pen.
    /// Invalid catalogs remain errors; OS package-query failures are never None.
    pub fn applicable_to_running_package(&self, package: Option<&str>) -> Result<Option<&Entry>> {
        self.validate()?;
        let policy = match package {
            None => SourcePolicy::Ordinary,
            Some(PAINT_PACKAGE) => SourcePolicy::InstalledPaint,
            _ => return Ok(None),
        };
        Ok(self.entries.iter().find(|entry| entry.policy() == policy))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_cannot_import_source_policy_from_titles_or_unknown_package() {
        let catalog = Catalog {
            schema_version: 4,
            entries: vec![],
        };
        assert_eq!(
            catalog.for_running_package(Some("Paint")),
            Err(Error::Invalid)
        );
        assert!(
            Catalog::decode("{\"schemaVersion\":4,\"entries\":[],\"title\":\"Paint\"}").is_err()
        );
    }
    #[test]
    fn absent_applicable_entry_is_not_catalog_or_provider_authority() -> Result<()> {
        let mut catalog = fixture()?;
        catalog
            .entries
            .retain(|entry| entry.policy() == SourcePolicy::Ordinary);
        assert_eq!(
            catalog.applicable_to_running_package(Some(PAINT_PACKAGE))?,
            None
        );
        assert_eq!(
            catalog.applicable_to_running_package(Some("foreign.package"))?,
            None
        );
        assert_eq!(
            catalog
                .applicable_to_running_package(None)?
                .map(Entry::policy),
            Some(SourcePolicy::Ordinary)
        );
        catalog.entries.clear();
        assert_eq!(
            catalog.applicable_to_running_package(None),
            Err(Error::Invalid)
        );
        Ok(())
    }
    fn fixture() -> Result<Catalog> {
        Ok(Catalog {
            schema_version: 4,
            entries: vec![
                Entry::Essential {
                    profile: super::super::essential::tests::fixture(
                        super::super::essential::Editor::Krita,
                    )?,
                },
                Entry::Essential {
                    profile: super::super::essential::tests::fixture(
                        super::super::essential::Editor::Paint,
                    )?,
                },
            ],
        })
    }
    #[test]
    fn actual_package_selection_never_falls_back_to_ordinary_or_missing_entry() -> Result<()> {
        let mut p = fixture()?;
        p.validate()?;
        assert_eq!(
            p.for_running_package(None)?.policy(),
            SourcePolicy::Ordinary
        );
        assert_eq!(
            p.for_running_package(Some(PAINT_PACKAGE))?.policy(),
            SourcePolicy::InstalledPaint
        );
        for name in [
            "",
            "Paint",
            "Microsoft.Paint_11.2605.81.1_x64__8wekyb3d8bbwe",
            "foreign",
        ] {
            assert_eq!(p.for_running_package(Some(name)), Err(Error::Unavailable));
        }
        p.entries.retain(|e| e.policy() == SourcePolicy::Ordinary);
        assert_eq!(
            p.for_running_package(Some(PAINT_PACKAGE)),
            Err(Error::Unavailable)
        );
        Ok(())
    }
    #[test]
    fn duplicate_source_policy_and_more_than_two_profiles_are_ambiguous() -> Result<()> {
        let p = fixture()?;
        let mut bad = p.clone();
        bad.entries[1] = bad.entries[0].clone();
        assert_eq!(bad.validate(), Err(Error::Invalid));
        bad = p;
        bad.entries.push(bad.entries[0].clone());
        assert_eq!(bad.validate(), Err(Error::Invalid));
        Ok(())
    }
    #[test]
    fn catalog_closed_bound_preserves_exact_nested_profile_validation() -> Result<()> {
        let p = fixture()?;
        let json = serde_json::to_string(&p).map_err(|_| Error::Invalid)?;
        assert_eq!(Catalog::decode(&json)?, p);
        assert!(
            Catalog::decode(&json.replace(
                "\"schemaVersion\":4",
                "\"schemaVersion\":4,\"schemaVersion\":4"
            ))
            .is_err()
        );
        assert_eq!(
            Catalog::decode(&" ".repeat(32 * 1024 + 1)),
            Err(Error::Limit)
        );
        let mut bad = p;
        let Entry::Essential { profile } = &mut bad.entries[0];
        profile.metadata.executable_blake3 = "f".repeat(64);
        assert_eq!(bad.validate(), Err(Error::Unavailable));
        Ok(())
    }
    #[test]
    fn valid_standalone_legacy_cannot_bypass_catalog_restore_and_budget_evidence() -> Result<()> {
        let p = super::super::essential::tests::fixture(super::super::essential::Editor::Krita)?;
        let legacy = super::super::live::LiveProfile {
            schema_version: 2,
            metadata: p.metadata,
            measured_tree_digest: "d".repeat(64),
            watched_preset: "b) Basic-5 Size Opacity".into(),
            watched_blending_mode: "Normal".into(),
            watched_preserve_alpha: false,
            watched_tool_ids: vec![super::super::live::BRUSH_ID.into()],
            actions: vec![super::super::live::ToolbarAction {
                action: 3,
                route: super::super::live::route_for(3)?,
                effect_receipt_digest: "e".repeat(64),
                guard_receipt_digest: "f".repeat(64),
            }],
        };
        legacy.validate()?;
        let profile = serde_json::to_value(legacy).map_err(|_| Error::Invalid)?;
        let json=serde_json::json!({"schemaVersion":4,"entries":[{"kind":"krita_toolbar","profile":profile}]}).to_string();
        assert_eq!(Catalog::decode(&json), Err(Error::Invalid));
        Ok(())
    }
}
