//! Incremental URI intake consumes the existing pre-open input admission.
//! The actual reader retires before its staging directory/permit can retire.
use super::{
    Error, GiantInputLimits, PickedOriginal, Result, RootOwnedPickedOriginal, cleanup, input,
    io_error, picked::PendingInputAdmission, reserve_picked_input,
};
use crate::Cancellation;
use std::{
    fs::{File, OpenOptions},
    io::{Seek, SeekFrom, Write},
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
use vw_stream::SessionBudget;

pub(super) const CHUNK: usize = 64 * 1024;

/// A fixed platform reader, created without opening the external source.
/// quarantine must retain BOTH this reader and the exact staged input. A failed
/// close, timeout, JNI error or caller cancellation never proves retirement.
pub(super) trait Reader: Sized {
    fn open(&mut self) -> Result<()>;
    fn read(&mut self, bytes: &mut [u8]) -> Result<usize>;
    fn close(&mut self) -> Result<()>;
    fn quarantine(self, input: StagedInput);
}

pub(super) struct StagedInput {
    file: Option<File>,
    work: Option<tempfile::TempDir>,
    admission: Option<PendingInputAdmission>,
    written: u64,
    original_limit: u64,
    disk_limit: u64,
}
impl StagedInput {
    pub(super) fn create(
        parent: &Path,
        limits: &GiantInputLimits,
        budget: SessionBudget,
        cancel: &Cancellation,
    ) -> Result<Self> {
        input::validate_limits(limits)?;
        cancel.check().map_err(Error::from)?;
        // Before every filesystem operation, Java owner allocation or resolver
        // open. This is the SAME two-input/4MiB admission as local picked files.
        let admission = reserve_picked_input(budget)?;
        let parent = input::checked_directory(parent)?;
        cancel.check().map_err(Error::from)?;
        let work = tempfile::Builder::new()
            .prefix("vw-giant-uri-")
            .tempdir_in(parent)
            .map_err(io_error)?;
        let mut staged = Self {
            file: None,
            work: Some(work),
            admission: Some(admission),
            written: 0,
            original_limit: limits.max_original_bytes,
            disk_limit: limits.max_work_disk_bytes,
        };
        let directory = staged.work.as_ref().ok_or(Error::Storage)?;
        input::admit_space(directory.path(), CHUNK as u64, &vw_store::RealFreeSpace)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(directory.path().join("picked.uri"))
            .map_err(io_error)?;
        staged.file = Some(file);
        Ok(staged)
    }
    fn append(&mut self, bytes: &[u8]) -> Result<()> {
        if bytes.is_empty() || bytes.len() > CHUNK {
            return Err(Error::Invalid);
        }
        let total = self
            .written
            .checked_add(bytes.len() as u64)
            .ok_or(Error::Limit)?;
        // Preparation copies the staged original once within the same root.
        // Its later decoded-geometry admission accounts for the larger envelope.
        if total > self.original_limit || total.checked_mul(2).is_none_or(|n| n > self.disk_limit) {
            return Err(Error::Limit);
        }
        let directory = self.work.as_ref().ok_or(Error::Storage)?;
        input::admit_space(
            directory.path(),
            bytes.len() as u64,
            &vw_store::RealFreeSpace,
        )?;
        self.file
            .as_mut()
            .ok_or(Error::Storage)?
            .write_all(bytes)
            .map_err(io_error)?;
        self.written = total;
        Ok(())
    }
    fn into_picked(mut self, limits: GiantInputLimits) -> Result<Arc<PickedOriginal>> {
        if self.written < 33 {
            return Err(Error::Invalid);
        }
        let file = self.file.as_mut().ok_or(Error::Storage)?;
        file.flush()
            .and_then(|()| file.sync_all())
            .map_err(io_error)?;
        if !file.metadata().map_err(io_error)?.is_file()
            || file.metadata().map_err(io_error)?.len() != self.written
        {
            return Err(Error::Stale);
        }
        file.seek(SeekFrom::Start(0)).map_err(io_error)?;
        // All fields were installed before source open. No fallible operation
        // separates the actual File/directory and their sole admission transfer.
        let (Some(file), Some(work), Some(admission)) =
            (self.file.take(), self.work.take(), self.admission.take())
        else {
            return Err(Error::Storage);
        };
        admission.attach(RootOwnedPickedOriginal { file, work }, limits)
    }
    pub(super) fn release(mut self) -> Result<()> {
        drop(self.file.take());
        match (self.work.take(), self.admission.take()) {
            (Some(work), Some(admission)) => {
                cleanup::OwnedDirectory::new(work, admission).cleanup()
            }
            (None, None) => Ok(()),
            _ => Err(Error::Storage),
        }
    }
}
impl Drop for StagedInput {
    fn drop(&mut self) {
        drop(self.file.take());
        if let (Some(work), Some(admission)) = (self.work.take(), self.admission.take()) {
            let _ = cleanup::OwnedDirectory::new(work, admission).cleanup();
        }
    }
}

struct Reading<R: Reader>(Option<(R, StagedInput)>);
impl<R: Reader> Drop for Reading<R> {
    fn drop(&mut self) {
        if let Some((mut reader, input)) = self.0.take() {
            if reader.close().is_err() {
                reader.quarantine(input);
            } else {
                let _ = input.release();
            }
        }
    }
}

/// Synchronous on an existing settled foreign worker. An uninterruptible OS
/// read retains that worker and this exact admission until read/close returns.
pub(super) fn copy_reader<R: Reader>(
    input: StagedInput,
    reader: R,
    limits: GiantInputLimits,
    timeout_ms: u64,
    cancel: &Cancellation,
) -> Result<Arc<PickedOriginal>> {
    let mut owner = Reading(Some((reader, input)));
    let result = (|| {
        if !(1..=120_000).contains(&timeout_ms) {
            return Err(Error::Limit);
        }
        let deadline = Instant::now()
            .checked_add(Duration::from_millis(timeout_ms))
            .ok_or(Error::Limit)?;
        let check = || {
            cancel.check().map_err(Error::from)?;
            if Instant::now() >= deadline {
                Err(Error::Limit)
            } else {
                Ok(())
            }
        };
        check()?;
        let (reader, input) = owner.0.as_mut().ok_or(Error::Storage)?;
        reader.open()?;
        let mut bytes = [0_u8; CHUNK];
        loop {
            check()?;
            let count = reader.read(&mut bytes)?;
            check()?;
            if count == 0 {
                break;
            }
            if count > bytes.len() {
                return Err(Error::Invalid);
            }
            input.append(&bytes[..count])?;
        }
        Ok(())
    })();
    let (mut reader, input) = owner.0.take().ok_or(Error::Storage)?;
    if let Err(error) = reader.close() {
        reader.quarantine(input);
        return Err(error);
    }
    if let Err(error) = result {
        return match input.release() {
            Ok(()) => Err(error),
            Err(cleanup) => Err(cleanup),
        };
    }
    if let Err(error) = cancel.check() {
        return match input.release() {
            Ok(()) => Err(Error::from(error)),
            Err(cleanup) => Err(cleanup),
        };
    }
    input.into_picked(limits)
}

#[cfg(test)]
mod tests;
