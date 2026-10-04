use crate::WorkflowBinding;
#[derive(Clone, Debug, thiserror::Error, uniffi::Error)]
pub enum PackageError {
    #[error("invalid package input")]
    Invalid,
    #[error("package or caller memory limit exceeded")]
    Limit,
    #[error("package source revision changed")]
    Stale,
    #[error("an exact original is missing or changed")]
    Original,
    #[error("package needs explicit depth conversion")]
    Depth,
    #[error("package format or installed client capability unsupported")]
    Unsupported,
    #[error("package or catalog integrity failure")]
    Integrity,
    #[error("package identity is already bound to different bytes")]
    Identity,
    #[error("package catalog storage failure; inspect the durable catalog")]
    Storage,
    #[error("package operation capacity exhausted")]
    Busy,
    #[error("package owner is closed")]
    Closed,
    #[error("package operation cancelled before publication")]
    Cancelled,
}
pub type PackageResult<T> = std::result::Result<T, PackageError>;
#[derive(Clone, Debug, uniffi::Enum)]
pub enum PackageTarget {
    ClaudeModern { model: String },
    ClaudeLegacy { model: String },
    OpenAiResponses { model: String, max_long_edge: u32 },
    Gemini { model: String, max_long_edge: u32 },
    Generic { max_long_edge: u32 },
}
impl PackageTarget {
    pub(super) fn native(&self) -> PackageResult<vw_package::Target> {
        use vw_package::{ClaudeTier, Target};
        let model = match self {
            Self::ClaudeModern { model }
            | Self::ClaudeLegacy { model }
            | Self::OpenAiResponses { model, .. }
            | Self::Gemini { model, .. } => Some(model),
            Self::Generic { .. } => None,
        };
        if model.is_some_and(|m| m.is_empty() || m.len() > 256 || m.chars().any(char::is_control)) {
            return Err(PackageError::Invalid);
        }
        let target = match self {
            Self::ClaudeModern { model } => Target::Claude {
                model: model.clone(),
                tier: ClaudeTier::Modern2576,
            },
            Self::ClaudeLegacy { model } => Target::Claude {
                model: model.clone(),
                tier: ClaudeTier::Legacy1568,
            },
            Self::OpenAiResponses {
                model,
                max_long_edge,
            } => Target::OpenAiResponses {
                model: model.clone(),
                max_long_edge: *max_long_edge,
            },
            Self::Gemini {
                model,
                max_long_edge,
            } => Target::Gemini {
                model: model.clone(),
                max_long_edge: *max_long_edge,
            },
            Self::Generic { max_long_edge } => Target::Generic {
                max_long_edge: *max_long_edge,
            },
        };
        if !(256..=8192).contains(&target.max_long_edge())
            || target
                .model()
                .is_some_and(|v| v.is_empty() || v.len() > 256 || v.chars().any(char::is_control))
        {
            return Err(PackageError::Invalid);
        }
        Ok(target)
    }
}
/// Codex is deliberately not a caller-asserted target. A separate installed
/// schema adapter must mint its opaque capability before that target is exposed.
#[derive(Clone, Debug, uniffi::Record)]
pub struct PackageCompileOptions {
    pub binding: WorkflowBinding,
    pub package_id: String,
    pub created_at_ms: i64,
    pub target: PackageTarget,
    pub semantic_snapshot_id: Option<String>,
    pub include_window_title: bool,
    pub assume_untagged_srgb: bool,
    pub allow_depth_reduction: bool,
    pub memory_budget_bytes: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PackageImageInfo {
    pub id: String,
    pub role: String,
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub encoded_bytes: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PackageInfo {
    pub package_id: String,
    pub target: String,
    pub model: Option<String>,
    pub binding: WorkflowBinding,
    pub source_asset_id: String,
    pub manifest_sha256: String,
    pub created_at: String,
    pub total_bytes: u64,
    pub marker_count: u32,
    pub images: Vec<PackageImageInfo>,
    pub inline_images_available: bool,
    pub includes_window_title: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PublishedPackage {
    pub info: PackageInfo,
    pub directory: String,
}
pub(super) fn describe(p: &vw_package::Package) -> PackageResult<PackageInfo> {
    let m = p.manifest();
    let mut aggregate = 0u64;
    let images = m
        .images
        .iter()
        .map(|i| {
            let n = p.file(&i.path).ok_or(PackageError::Integrity)?.len() as u64;
            aggregate = aggregate.checked_add(n).ok_or(PackageError::Limit)?;
            Ok(PackageImageInfo {
                id: i.id.clone(),
                role: i.role.clone(),
                path: i.path.clone(),
                width: i.width,
                height: i.height,
                encoded_bytes: n,
            })
        })
        .collect::<PackageResult<Vec<_>>>()?;
    Ok(PackageInfo {
        package_id: m.package_id.to_string(),
        target: m.compiled_for.target.clone(),
        model: m.compiled_for.model.clone(),
        binding: WorkflowBinding {
            project_id: m.source.project_id.to_string(),
            document_id: m.source.document_id.to_string(),
            host_seq: m.extensions.host_seq,
            state_hash: m.extensions.state_hash.clone(),
        },
        source_asset_id: m.source.asset_hash.clone(),
        manifest_sha256: p.manifest_sha256().into(),
        created_at: m.created_at.clone(),
        total_bytes: p.total_bytes() as u64,
        marker_count: m.markers.len() as u32,
        images,
        inline_images_available: aggregate <= 9 * 1024 * 1024,
        includes_window_title: m
            .source
            .capture
            .as_ref()
            .is_some_and(|c| c.window_title.is_some()),
    })
}
