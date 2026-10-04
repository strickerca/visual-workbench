use crate::{
    DocumentView, EditMetadata, EditPlan, EntryMethod, Error, InstructionEdit, Result, Role,
    SourceBinding, text,
};
use vw_model::Id;

/// Transient keyboard/recognizer state bound to one exact field and revision.
/// Partial results never generate canonical ops. Platform callbacks must retain
/// this session ID and focus generation; a new field uses a new session.
pub struct EntrySession {
    id: Id,
    binding: SourceBinding,
    focus_generation: u64,
    edit: InstructionEdit,
    last_sequence: Option<u64>,
    closed: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryUpdate {
    Applied,
    Duplicate,
    Stale,
}
impl EntrySession {
    pub fn begin(
        view: &DocumentView<'_>,
        instruction: &Id,
        session: Id,
        focus_generation: u64,
        method: EntryMethod,
    ) -> Result<Self> {
        let value = &view.instruction(instruction)?.definition;
        Ok(Self {
            id: session,
            binding: view.binding().clone(),
            focus_generation,
            edit: InstructionEdit {
                instruction_id: instruction.clone(),
                targets: value
                    .target_object_ids
                    .iter()
                    .map(|v| Id::from_proto(Some(v)))
                    .collect::<std::result::Result<_, _>>()?,
                role: Role::from_canonical(value.role)?,
                text: value.text.clone(),
                entry_method: method,
                language: value.language.clone(),
            },
            last_sequence: None,
            closed: false,
        })
    }
    pub fn id(&self) -> &Id {
        &self.id
    }
    pub fn binding(&self) -> &SourceBinding {
        &self.binding
    }
    pub fn instruction_id(&self) -> &Id {
        &self.edit.instruction_id
    }
    /// Literal draft text remains available after stale-revision refusal so the
    /// UI can offer an explicit refresh/reapply instead of discarding dictation.
    pub fn draft(&self) -> &str {
        &self.edit.text
    }
    pub fn method(&self) -> EntryMethod {
        self.edit.entry_method
    }
    pub fn update(
        &mut self,
        session: &Id,
        focus_generation: u64,
        sequence: u64,
        value: &str,
    ) -> Result<EntryUpdate> {
        self.check(session, focus_generation)?;
        text(value)?;
        if let Some(previous) = self.last_sequence {
            if sequence < previous {
                return Ok(EntryUpdate::Stale);
            }
            if sequence == previous {
                return if value == self.edit.text {
                    Ok(EntryUpdate::Duplicate)
                } else {
                    Err(Error::Invalid("entry sequence reused"))
                };
            }
        }
        self.edit.text = value.to_owned();
        self.last_sequence = Some(sequence);
        Ok(EntryUpdate::Applied)
    }
    /// Final recognition is still a local draft until the caller explicitly
    /// invokes this method. No confidence score or platform result auto-sends.
    /// Borrowed finalization permits the platform boundary to admit transaction
    /// clones and cancellation before consuming the local draft. No state changes.
    pub fn prepare(
        &self,
        session: &Id,
        focus_generation: u64,
        view: &DocumentView<'_>,
        meta: EditMetadata,
    ) -> Result<EditPlan> {
        self.check(session, focus_generation)?;
        if self.binding != *view.binding() {
            return Err(Error::Stale);
        }
        view.set_instruction(meta, self.edit.clone())
    }
    /// Seal only after the caller has admitted and retained its prepared plan.
    /// This changes transient entry state only; literal text remains readable.
    pub fn seal(&mut self, session: &Id, focus_generation: u64) -> Result<()> {
        self.check(session, focus_generation)?;
        self.closed = true;
        Ok(())
    }
    pub fn commit(
        &mut self,
        session: &Id,
        focus_generation: u64,
        view: &DocumentView<'_>,
        meta: EditMetadata,
    ) -> Result<EditPlan> {
        let plan = self.prepare(session, focus_generation, view, meta)?;
        self.seal(session, focus_generation)?;
        Ok(plan)
    }
    pub fn cancel(&mut self) {
        self.closed = true;
        self.edit.text.clear();
    }
    fn check(&self, session: &Id, focus_generation: u64) -> Result<()> {
        if self.closed || session != &self.id || focus_generation != self.focus_generation {
            return Err(Error::Closed);
        }
        Ok(())
    }
}
