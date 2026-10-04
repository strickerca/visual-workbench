use crate::{ObjectStyle, Point, QueryRect, WorkflowBinding};

#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum InstructionRole {
    None,
    Change,
    Preserve,
    Reference,
    Explain,
}
impl InstructionRole {
    pub(super) fn core(self) -> vw_instructions::Role {
        match self {
            Self::None => vw_instructions::Role::None,
            Self::Change => vw_instructions::Role::Change,
            Self::Preserve => vw_instructions::Role::Preserve,
            Self::Reference => vw_instructions::Role::Reference,
            Self::Explain => vw_instructions::Role::Explain,
        }
    }
    pub(super) fn from_core(value: vw_instructions::Role) -> Self {
        match value {
            vw_instructions::Role::None => Self::None,
            vw_instructions::Role::Change => Self::Change,
            vw_instructions::Role::Preserve => Self::Preserve,
            vw_instructions::Role::Reference => Self::Reference,
            vw_instructions::Role::Explain => Self::Explain,
        }
    }
}
#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum InstructionEntryMethod {
    PcKeyboard,
    PhoneKeyboard,
    Voice,
    Handwriting,
}
impl InstructionEntryMethod {
    pub(super) fn core(self) -> vw_instructions::EntryMethod {
        match self {
            Self::PcKeyboard => vw_instructions::EntryMethod::PcKeyboard,
            Self::PhoneKeyboard => vw_instructions::EntryMethod::PhoneKeyboard,
            Self::Voice => vw_instructions::EntryMethod::Voice,
            Self::Handwriting => vw_instructions::EntryMethod::Handwriting,
        }
    }
    pub(super) fn from_core(value: vw_instructions::EntryMethod) -> Self {
        match value {
            vw_instructions::EntryMethod::PcKeyboard => Self::PcKeyboard,
            vw_instructions::EntryMethod::PhoneKeyboard => Self::PhoneKeyboard,
            vw_instructions::EntryMethod::Voice => Self::Voice,
            vw_instructions::EntryMethod::Handwriting => Self::Handwriting,
        }
    }
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct InstructionRow {
    pub instruction_id: String,
    pub target_ids: Vec<String>,
    pub role: InstructionRole,
    pub text: String,
    pub entry_method: InstructionEntryMethod,
    pub language: String,
    pub updated_at_ms: i64,
    pub detached: bool,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct MarkerRow {
    pub object_id: String,
    pub instruction_id: Option<String>,
    pub number: u32,
    pub point_document: Point,
    pub bounds_document: Option<QueryRect>,
    pub element_eids: Vec<String>,
    pub hidden: bool,
    pub layer_visible: bool,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct InstructionDocument {
    pub binding: WorkflowBinding,
    pub instructions: Vec<InstructionRow>,
    pub markers: Vec<MarkerRow>,
    /// Duplicate/missing numbers, links or roles must be reconciled explicitly.
    pub needs_reconciliation: bool,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct InstructionProjection {
    pub binding: WorkflowBinding,
    pub json: Vec<u8>,
    pub prompt_fragment: String,
}
#[derive(Clone, Debug, uniffi::Enum)]
pub enum InstructionCommand {
    PlaceMarker {
        object_id: String,
        instruction_id: String,
        layer_id: String,
        point: Point,
        bounds: Option<QueryRect>,
        element_eids: Vec<String>,
        style: ObjectStyle,
        role: InstructionRole,
        text: String,
        entry_method: InstructionEntryMethod,
        language: String,
    },
    SetInstruction {
        instruction_id: String,
        target_ids: Vec<String>,
        role: InstructionRole,
        text: String,
        entry_method: InstructionEntryMethod,
        language: String,
    },
    DeleteMarker {
        object_id: String,
    },
    DeleteInstruction {
        instruction_id: String,
    },
}
#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum DraftUpdateStatus {
    Applied,
    Duplicate,
    Stale,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct InstructionDraftValue {
    pub session_id: String,
    pub instruction_id: String,
    pub binding: WorkflowBinding,
    pub text: String,
    pub entry_method: InstructionEntryMethod,
}
