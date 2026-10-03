use crate::{Error, Result, config::ProviderConfig};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum State {
    Reserved,
    Attempted,
    Settled,
    NotSent,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    request_id: String,
    provider_sha256: String,
    utc_day: u64,
    reserved_microusd: u64,
    actual_microusd: Option<u64>,
    state: State,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ledger {
    entries: Vec<Entry>,
}

/// Holds the exclusive file lock across the single network attempt. An interrupted
/// or ambiguous attempt remains reserved and blocks further sends for inspection.
pub struct Reservation {
    file: File,
    ledger: Ledger,
    index: usize,
    pub daily_soft_budget_exceeded: bool,
}

impl Reservation {
    pub fn begin(
        path: &Path,
        config: &ProviderConfig,
        request_id: &str,
        confirmed_id: &str,
        utc_day: u64,
    ) -> Result<Self> {
        config.validate()?;
        let estimated = config.estimate()?.ok_or(Error::Confirmation)?;
        if request_id != confirmed_id
            || request_id.len() != 64
            || !request_id.bytes().all(|v| v.is_ascii_hexdigit())
        {
            return Err(Error::Confirmation);
        }
        if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink() || !m.is_file())
        {
            return Err(Error::Invalid("budget ledger must be a regular file"));
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT: do not follow a final link.
        }
        let mut file = options.open(path)?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if file.metadata()?.file_attributes() & 0x400 != 0 {
                return Err(Error::Invalid("budget reparse point"));
            }
        }
        fs2::FileExt::try_lock_exclusive(&file)?;
        if file.metadata()?.len() > 131_072 {
            return Err(Error::Limit("budget ledger bytes"));
        }
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(131_073)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 131_072 {
            return Err(Error::Limit("budget ledger bytes"));
        }
        let mut ledger: Ledger = if bytes.is_empty() {
            Ledger::default()
        } else {
            serde_json::from_slice(&bytes)?
        };
        if ledger.entries.len() >= 256
            || ledger.entries.iter().any(|entry| {
                matches!(entry.state, State::Reserved | State::Attempted)
                    || (entry.request_id == request_id && entry.state != State::NotSent)
                    || entry.reserved_microusd == 0
                    || !valid_hash(&entry.request_id)
                    || !valid_hash(&entry.provider_sha256)
                    || match entry.state {
                        State::Settled => entry.actual_microusd.is_none_or(|cost| cost == 0),
                        _ => entry.actual_microusd.is_some(),
                    }
            })
        {
            return Err(Error::Invalid(
                "unresolved, duplicate or corrupt budget reservation",
            ));
        }
        let charge = |entry: &Entry| match entry.state {
            State::NotSent => 0,
            _ => entry.actual_microusd.unwrap_or(entry.reserved_microusd),
        };
        let total = ledger.entries.iter().try_fold(0u64, |sum, entry| {
            sum.checked_add(charge(entry)).ok_or(Error::Budget)
        })?;
        if total.checked_add(estimated).ok_or(Error::Budget)? > config.spike_budget_microusd {
            return Err(Error::Budget);
        }
        let daily = ledger
            .entries
            .iter()
            .filter(|entry| entry.utc_day == utc_day)
            .try_fold(0u64, |sum, entry| {
                sum.checked_add(charge(entry)).ok_or(Error::Budget)
            })?;
        let daily_soft_budget_exceeded =
            daily.checked_add(estimated).ok_or(Error::Budget)? > config.daily_soft_budget_microusd;
        ledger.entries.push(Entry {
            request_id: request_id.into(),
            provider_sha256: provider_hash(config)?,
            utc_day,
            reserved_microusd: estimated,
            actual_microusd: None,
            state: State::Reserved,
        });
        let index = ledger.entries.len() - 1;
        let mut reservation = Self {
            file,
            ledger,
            index,
            daily_soft_budget_exceeded,
        };
        reservation.persist()?;
        Ok(reservation)
    }

    fn persist(&mut self) -> Result<()> {
        let bytes = serde_json::to_vec(&self.ledger)?;
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(&bytes)?;
        self.file.set_len(bytes.len() as u64)?;
        self.file.sync_all()?;
        Ok(())
    }

    pub fn request_id(&self) -> &str {
        &self.ledger.entries[self.index].request_id
    }

    pub(crate) fn ensure_unused(&self) -> Result<()> {
        if self.ledger.entries[self.index].state != State::Reserved {
            return Err(Error::AttemptConsumed);
        }
        Ok(())
    }

    /// Durably consume the one allowed attempt before entering any path that
    /// could issue a POST. The same reservation cannot be reused in this process;
    /// a dropped/interrupted attempted entry blocks reopening the live ledger.
    /// Offline tests may call this boundary directly without reading credentials.
    pub fn begin_attempt(&mut self, request_id: &str, config: &ProviderConfig) -> Result<()> {
        self.ensure_unused()?;
        config.validate()?;
        let entry = &mut self.ledger.entries[self.index];
        if entry.request_id != request_id
            || entry.provider_sha256 != provider_hash(config)?
            || config.estimate()? != Some(entry.reserved_microusd)
        {
            return Err(Error::Confirmation);
        }
        entry.state = State::Attempted;
        self.persist()
    }

    /// Call only when no request was sent (for example credential read failed).
    pub fn not_sent(mut self) -> Result<()> {
        self.ledger.entries[self.index].state = State::NotSent;
        self.persist()
    }

    /// None keeps an unresolved reservation. It cannot silently enable another send.
    pub fn settle(mut self, actual_microusd: Option<u64>) -> Result<()> {
        if self.ledger.entries[self.index].state != State::Attempted || actual_microusd == Some(0) {
            return Err(Error::Invalid(
                "settlement requires an attempted request and nonzero charge",
            ));
        }
        if let Some(actual) = actual_microusd {
            self.ledger.entries[self.index].actual_microusd = Some(actual);
            self.ledger.entries[self.index].state = State::Settled;
        }
        self.persist()
    }
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
fn provider_hash(config: &ProviderConfig) -> Result<String> {
    Ok(crate::sha256(&serde_json::to_vec(&serde_json::to_value(
        config,
    )?)?))
}
