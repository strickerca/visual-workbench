use super::*;
use crate::worker::Worker;
use std::sync::{
    Mutex, MutexGuard, TryLockError, Weak,
    atomic::{AtomicBool, Ordering},
};

// Only the project worker may wait on entry preparation. Synchronous UI calls
// never wait for codec/accounting/hash work held by the worker.
fn try_entry(
    entry: &Mutex<Option<vw_instructions::EntrySession>>,
) -> WorkflowResult<MutexGuard<'_, Option<vw_instructions::EntrySession>>> {
    entry.try_lock().map_err(|error| match error {
        TryLockError::WouldBlock => WorkflowError::Backpressure,
        TryLockError::Poisoned(_) => WorkflowError::Closed,
    })
}
fn clear_entry(entry: &mut Option<vw_instructions::EntrySession>) {
    if let Some(value) = entry.as_mut() {
        value.cancel();
    }
    *entry = None;
}
fn dispose_entry(entry: &Mutex<Option<vw_instructions::EntrySession>>, disposed: &AtomicBool) {
    disposed.store(true, Ordering::Release);
    // A worker holding the entry owns deferred destruction; never block the UI.
    if let Ok(mut entry) = entry.try_lock() {
        clear_entry(&mut entry);
    }
}
fn with_draft_entry<T>(
    entry: &Mutex<Option<vw_instructions::EntrySession>>,
    disposed: &AtomicBool,
    job: impl FnOnce(&mut Option<vw_instructions::EntrySession>) -> WorkflowResult<T>,
) -> WorkflowResult<T> {
    let mut entry = entry.lock().map_err(|_| WorkflowError::Closed)?;
    if disposed.load(Ordering::Acquire) {
        clear_entry(&mut entry);
        return Err(WorkflowError::Closed);
    }
    let result = job(&mut entry);
    if disposed.load(Ordering::Acquire) {
        clear_entry(&mut entry);
        return Err(WorkflowError::Closed);
    }
    result
}

#[derive(uniffi::Object)]
pub struct InstructionDraft {
    project: Weak<ProjectSession>,
    worker: Worker<crate::project::ProjectState>,
    closed: Arc<AtomicBool>,
    disposed: Arc<AtomicBool>,
    entry: Arc<Mutex<Option<vw_instructions::EntrySession>>>,
    binding: WorkflowBinding,
    memory_budget_bytes: u64,
    _permit: HandlePermit,
}
impl InstructionDraft {
    fn check(&self) -> WorkflowResult<()> {
        if self.closed.load(Ordering::Acquire) || self.disposed.load(Ordering::Acquire) {
            return Err(WorkflowError::Closed);
        }
        Ok(())
    }
}
#[uniffi::export]
impl InstructionDraft {
    /// Provenance records which platform adapter supplied text; it does not
    /// establish microphone/handwriting permission, recognition or accuracy.
    pub fn value(&self) -> WorkflowResult<InstructionDraftValue> {
        self.check()?;
        let entry = try_entry(&self.entry)?;
        let entry = entry.as_ref().ok_or(WorkflowError::Closed)?;
        Ok(InstructionDraftValue {
            session_id: entry.id().to_string(),
            instruction_id: entry.instruction_id().to_string(),
            binding: self.binding.clone(),
            text: entry.draft().to_owned(),
            entry_method: InstructionEntryMethod::from_core(entry.method()),
        })
    }
    pub fn update(
        &self,
        session_id: String,
        focus_generation: u64,
        sequence: u64,
        text: String,
    ) -> WorkflowResult<DraftUpdateStatus> {
        self.check()?;
        bounded_id(&session_id)?;
        bounded_text(&text, "")?;
        let mut entry = try_entry(&self.entry)?;
        Ok(
            match entry.as_mut().ok_or(WorkflowError::Closed)?.update(
                &Id::try_from(session_id)?,
                focus_generation,
                sequence,
                &text,
            )? {
                vw_instructions::EntryUpdate::Applied => DraftUpdateStatus::Applied,
                vw_instructions::EntryUpdate::Duplicate => DraftUpdateStatus::Duplicate,
                vw_instructions::EntryUpdate::Stale => DraftUpdateStatus::Stale,
            },
        )
    }
    /// Explicit finalization prepares immutable ops; it still does not commit.
    /// A stale result keeps the literal draft available for explicit reapply.
    pub async fn prepare(
        &self,
        session_id: String,
        focus_generation: u64,
        metadata: WorkflowMetadata,
        cancellation: Arc<Cancellation>,
    ) -> WorkflowResult<Arc<WorkflowPlan>> {
        self.check()?;
        bounded_id(&session_id)?;
        metadata.validate()?;
        let permit = HandlePermit::acquire()?;
        let project = self.project.upgrade().ok_or(WorkflowError::Closed)?;
        let entry = self.entry.clone();
        let disposed = self.disposed.clone();
        let binding = self.binding.clone();
        let budget = self.memory_budget_bytes;
        self.worker
            .call(move |state| {
                Ok((|| {
                    check_live(&project.closed, &cancellation)?;
                    admit(state, budget, true, INSTRUCTION_WORK)?;
                    check_metadata(state, &metadata)?;
                    let view = view(state, &binding)?;
                    with_draft_entry(&entry, &disposed, |entry| {
                        let entry = entry.as_mut().ok_or(WorkflowError::Closed)?;
                        let session_id = Id::try_from(session_id)?;
                        let planned = entry.prepare(
                            &session_id,
                            focus_generation,
                            &view,
                            metadata.instruction()?,
                        )?;
                        admit_transaction(state, planned.transaction(), budget, INSTRUCTION_WORK)?;
                        cancellation.check()?;
                        if disposed.load(Ordering::Acquire) {
                            return Err(WorkflowError::Closed);
                        }
                        let opaque = WorkflowPlan::new(
                            &project,
                            state,
                            binding,
                            planned.transaction().clone(),
                            planned.next_lamport(),
                            budget,
                            INSTRUCTION_WORK,
                            permit,
                        )?;
                        check_live(&project.closed, &cancellation)?;
                        if disposed.load(Ordering::Acquire) {
                            return Err(WorkflowError::Closed);
                        }
                        entry.seal(&session_id, focus_generation)?;
                        Ok(opaque)
                    })
                })())
            })
            .await?
    }
    pub fn dispose(&self) {
        dispose_entry(&self.entry, &self.disposed);
    }
}
impl Drop for InstructionDraft {
    fn drop(&mut self) {
        self.dispose();
    }
}
#[uniffi::export]
impl ProjectSession {
    #[allow(clippy::too_many_arguments)]
    pub async fn begin_instruction_draft(
        self: Arc<Self>,
        binding: WorkflowBinding,
        instruction_id: String,
        session_id: String,
        focus_generation: u64,
        entry_method: InstructionEntryMethod,
        memory_budget_bytes: u64,
        cancellation: Arc<Cancellation>,
    ) -> WorkflowResult<Arc<InstructionDraft>> {
        self.check_open()?;
        binding.validate()?;
        bounded_id(&instruction_id)?;
        bounded_id(&session_id)?;
        let permit = HandlePermit::acquire()?;
        let project = self.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    check_live(&project.closed, &cancellation)?;
                    admit(state, memory_budget_bytes, false, INSTRUCTION_WORK)?;
                    let view = view(state, &binding)?;
                    let entry = vw_instructions::EntrySession::begin(
                        &view,
                        &Id::try_from(instruction_id)?,
                        Id::try_from(session_id)?,
                        focus_generation,
                        entry_method.core(),
                    )?;
                    cancellation.check()?;
                    Ok(Arc::new(InstructionDraft {
                        project: Arc::downgrade(&project),
                        worker: project.worker.clone(),
                        closed: project.closed.clone(),
                        disposed: Arc::new(AtomicBool::new(false)),
                        entry: Arc::new(Mutex::new(Some(entry))),
                        binding,
                        memory_budget_bytes,
                        _permit: permit,
                    }))
                })())
            })
            .await?
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod lifecycle_tests {
    use super::*;
    use std::{
        sync::{atomic::AtomicUsize, mpsc},
        time::Duration,
    };

    #[test]
    fn synchronous_disposal_does_not_wait_for_worker_entry_lock_and_blocks_queued_publication() {
        let entry = Mutex::new(None);
        let disposed = AtomicBool::new(false);
        let calls = AtomicUsize::new(0);
        let held = entry.lock().unwrap();
        assert!(matches!(
            try_entry(&entry),
            Err(WorkflowError::Backpressure)
        ));
        std::thread::scope(|scope| {
            let (done, received) = mpsc::channel();
            let (entry_ref, disposed_ref, calls_ref) = (&entry, &disposed, &calls);
            let disposer = scope.spawn(move || {
                dispose_entry(entry_ref, disposed_ref);
                done.send(()).unwrap();
            });
            let worker = scope.spawn(move || {
                with_draft_entry(entry_ref, disposed_ref, |_| {
                    calls_ref.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
            });
            let prompt = received.recv_timeout(Duration::from_secs(2));
            // Always unlock before joining, including a regressed blocking
            // implementation; the test itself must not strand a worker.
            drop(held);
            disposer.join().unwrap();
            assert!(prompt.is_ok(), "dispose blocked on worker-owned entry");
            assert!(matches!(worker.join().unwrap(), Err(WorkflowError::Closed)));
        });
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn disposal_during_owned_preparation_drops_undelivered_result_and_settles_entry() {
        struct Late<'a>(&'a AtomicUsize);
        impl Drop for Late<'_> {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let entry = Mutex::new(None);
        let disposed = AtomicBool::new(false);
        let dropped = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            let (started, start_rx) = mpsc::channel();
            let (release, release_rx) = mpsc::channel();
            let (entry_ref, disposed_ref, dropped_ref) = (&entry, &disposed, &dropped);
            let worker = scope.spawn(move || {
                with_draft_entry(entry_ref, disposed_ref, |_| {
                    started.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(4)).unwrap();
                    Ok(Late(dropped_ref))
                })
            });
            start_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            dispose_entry(&entry, &disposed);
            assert_eq!(dropped.load(Ordering::SeqCst), 0);
            release.send(()).unwrap();
            assert!(matches!(worker.join().unwrap(), Err(WorkflowError::Closed)));
        });
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
        assert!(entry.try_lock().unwrap().is_none());
    }
}
