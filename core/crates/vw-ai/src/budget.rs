//! App-owned durable accounting. An attempted request never becomes retryable
//! through cancellation, timeout, process restart, or an absent provider receipt.
use crate::{
    Error, Result, ReviewSummary,
    config::{ProviderConfig, Tokens},
    valid_hash,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, limits::Limit, params};
use std::{
    fs::{self, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

const APPLICATION_ID: i64 = 0x56574149;
const MAX_ENTRIES: i64 = 4096;
const PAGE_BYTES: u64 = 4096;
const MAX_PAGES: i64 = 2048;
/// Fixed app-ledger bounds, including existing files before SQLite opens them.
pub const MAX_LEDGER_BYTES: u64 = PAGE_BYTES * MAX_PAGES as u64;
/// Bounded disk recovery work, not a RAM allocation or a row-count-derived cap.
/// A delayed checkpoint can retain many rewrites of the same small database.
pub const MAX_LEDGER_WAL_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_SHM_BYTES: u64 = 16 * 1024 * 1024;
const MAX_SQLITE_VALUE_BYTES: i32 = 8192;
const META_SCHEMA: &str = "CREATE TABLE meta (id INTEGER PRIMARY KEY CHECK(id=1), format INTEGER NOT NULL CHECK(format=1), sequence INTEGER NOT NULL CHECK(sequence>=0), last_day INTEGER NOT NULL CHECK(last_day>=0)) STRICT";
const ENTRIES_SCHEMA: &str = "CREATE TABLE entries (request TEXT PRIMARY KEY, provider TEXT NOT NULL, utc_day INTEGER NOT NULL CHECK(utc_day>0), quote INTEGER NOT NULL CHECK(quote>0), state INTEGER NOT NULL CHECK(state BETWEEN 0 AND 3), actual INTEGER, sequence INTEGER UNIQUE NOT NULL CHECK(sequence>0), CHECK((state=2 AND actual IS NOT NULL AND actual>=0) OR (state<>2 AND actual IS NULL))) STRICT";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BudgetPolicy {
    pub daily_soft_limit_microusd: u64,
}
impl Default for BudgetPolicy {
    fn default() -> Self {
        Self {
            daily_soft_limit_microusd: 5_000_000,
        }
    }
}

/// Construct only in response to the owner's explicit Send action on this exact
/// review. A caller supplies the current UTC day (days since Unix epoch), never
/// a project-selected clock. Acknowledging a warning is separate from Send.
pub struct Confirmation {
    request: String,
    quote: u64,
    utc_day: u32,
    acknowledge_soft_limit: bool,
}
impl Confirmation {
    pub fn explicit_send(
        review: &ReviewSummary,
        utc_day: u32,
        acknowledge_soft_limit: bool,
    ) -> Result<Self> {
        if utc_day == 0 || utc_day > 1_000_000 {
            return Err(Error::Invalid("UTC day"));
        }
        let quote = review
            .estimated_microusd()
            .filter(|&n| n > 0)
            .ok_or(Error::Confirmation)?;
        Ok(Self {
            request: review.request_id().into(),
            quote,
            utc_day,
            acknowledge_soft_limit,
        })
    }
}

/// Deliberately not Clone/Serialize. Produced only after the Attempted state is
/// durably committed, consumed when constructing the actual transport payload.
pub struct AttemptPermit {
    pub(crate) request: String,
    pub(crate) provider: String,
    pub(crate) quote: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryState {
    Reserved,
    Attempted,
    Settled,
    NotSent,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LedgerEntry {
    pub request_id: String,
    pub provider_fingerprint: String,
    pub utc_day: u32,
    pub estimated_microusd: u64,
    pub actual_microusd: Option<u64>,
    pub state: EntryState,
}

mod sealed {
    pub trait Sealed {}
}
/// This sealed contract commits each transition durably before returning. An
/// adapter may not obtain credentials or initiate network IO before consuming
/// begin_attempt's permit. No method refunds an already attempted request.
pub trait BudgetLedger: sealed::Sealed {
    fn reserve(
        &mut self,
        review: &ReviewSummary,
        confirmation: Confirmation,
        policy: BudgetPolicy,
    ) -> Result<()>;
    fn begin_attempt(&mut self, review: &ReviewSummary) -> Result<AttemptPermit>;
    fn cancel_unattempted(&mut self, request_id: &str) -> Result<()>;
    fn settle(
        &mut self,
        request_id: &str,
        provider: &ProviderConfig,
        tokens: Tokens,
    ) -> Result<u64>;
    fn entry(&self, request_id: &str) -> Result<Option<LedgerEntry>>;
    fn spent_on(&self, utc_day: u32) -> Result<u64>;
}

/// Open on a dedicated blocking worker. The fixed application-private ledger
/// path belongs to a platform adapter, never a document or a user-chosen export.
/// `open` never creates or repairs a missing/corrupt ledger. Its separate first-
/// install `create` operation must not be used as error recovery.
pub struct SqliteBudgetLedger {
    connection: Connection,
    path: PathBuf,
}
impl SqliteBudgetLedger {
    pub fn create(path: &Path) -> Result<Self> {
        validate_path(path, false)?;
        // A crash after this exclusive create leaves an invalid file. open then
        // fails closed; it does not erase uncertain expenditure history.
        let file = OpenOptions::new().write(true).create_new(true).open(path)?;
        file.sync_all()?;
        drop(file);
        let mut connection = connect(path, true)?;
        {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute_batch(META_SCHEMA)?;
            tx.execute_batch(ENTRIES_SCHEMA)?;
            tx.execute_batch(
                "INSERT INTO meta VALUES(1,1,0,0);
PRAGMA user_version=1;
PRAGMA application_id=1448558921;",
            )?;
            tx.commit()?;
        }
        let ledger = Self {
            connection,
            path: path.to_owned(),
        };
        ledger.validate()?;
        Ok(ledger)
    }
    pub fn open(path: &Path) -> Result<Self> {
        validate_path(path, true)?;
        let ledger = Self {
            connection: connect(path, false)?,
            path: path.to_owned(),
        };
        ledger.validate()?;
        Ok(ledger)
    }
    fn validate(&self) -> Result<()> {
        // One read transaction binds schema, typed preflight and integrity to the
        // same snapshot. Every mutation repeats the checks under its write lock.
        validate_path(&self.path, true)?;
        let tx = self.connection.unchecked_transaction()?;
        validate_connection(&tx, &self.path)?;
        let check: String = tx.query_row("PRAGMA integrity_check(1)", [], |r| r.get(0))?;
        if check != "ok" {
            return Err(Error::Storage);
        }
        tx.commit()?;
        Ok(())
    }
}

impl sealed::Sealed for SqliteBudgetLedger {}
impl BudgetLedger for SqliteBudgetLedger {
    fn reserve(
        &mut self,
        review: &ReviewSummary,
        confirmation: Confirmation,
        policy: BudgetPolicy,
    ) -> Result<()> {
        let quote = review
            .estimated_microusd()
            .filter(|&n| n > 0)
            .ok_or(Error::Confirmation)?;
        if confirmation.request != review.request_id() || confirmation.quote != quote {
            return Err(Error::Confirmation);
        }
        let quote_i = i64::try_from(quote).map_err(|_| Error::Limit("ledger charge"))?;
        if policy.daily_soft_limit_microusd == 0 {
            return Err(Error::Invalid("daily budget"));
        }
        validate_path(&self.path, true)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        validate_connection(&tx, &self.path)?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM entries WHERE request=?1)",
            [review.request_id()],
            |r| r.get(0),
        )?;
        if exists {
            return Err(Error::AttemptConsumed);
        }
        let unresolved: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM entries WHERE state IN (0,1))",
            [],
            |r| r.get(0),
        )?;
        if unresolved {
            return Err(Error::Unresolved);
        }
        let (sequence, last_day): (i64, i64) =
            tx.query_row("SELECT sequence,last_day FROM meta WHERE id=1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?;
        if i64::from(confirmation.utc_day) < last_day {
            return Err(Error::ClockRegression);
        }
        if sequence >= MAX_ENTRIES {
            return Err(Error::Limit(
                "ledger history; application migration required",
            ));
        }
        let spent = spent_on_connection(&tx, confirmation.utc_day)?;
        let exceeds = spent
            .checked_add(quote)
            .ok_or(Error::Limit("daily charge"))?
            > policy.daily_soft_limit_microusd;
        if exceeds && !confirmation.acknowledge_soft_limit {
            return Err(Error::SoftBudget);
        }
        require_changed(tx.execute("INSERT INTO entries(request,provider,utc_day,quote,state,actual,sequence) VALUES(?1,?2,?3,?4,0,NULL,?5)",params![review.request_id(),review.provider_fingerprint(),confirmation.utc_day,quote_i,sequence+1])?)?;
        require_changed(tx.execute(
            "UPDATE meta SET sequence=?1,last_day=?2 WHERE id=1 AND sequence=?3 AND last_day=?4",
            params![sequence + 1, confirmation.utc_day, sequence, last_day],
        )?)?;
        require_entry(
            &tx,
            &LedgerEntry {
                request_id: review.request_id().into(),
                provider_fingerprint: review.provider_fingerprint().into(),
                utc_day: confirmation.utc_day,
                estimated_microusd: quote,
                actual_microusd: None,
                state: EntryState::Reserved,
            },
        )?;
        let metadata: (i64, i64) =
            tx.query_row("SELECT sequence,last_day FROM meta WHERE id=1", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?;
        if metadata != (sequence + 1, i64::from(confirmation.utc_day)) {
            return Err(Error::Storage);
        }
        tx.commit()?;
        Ok(())
    }
    fn begin_attempt(&mut self, review: &ReviewSummary) -> Result<AttemptPermit> {
        validate_path(&self.path, true)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        validate_connection(&tx, &self.path)?;
        let mut entry = read_entry(&tx, review.request_id())?.ok_or(Error::Confirmation)?;
        if entry.state != EntryState::Reserved {
            return Err(Error::AttemptConsumed);
        }
        if entry.provider_fingerprint != review.provider_fingerprint()
            || Some(entry.estimated_microusd) != review.estimated_microusd()
        {
            return Err(Error::Confirmation);
        }
        require_changed(tx.execute(
            "UPDATE entries SET state=1 WHERE request=?1 AND state=0",
            [review.request_id()],
        )?)?;
        entry.state = EntryState::Attempted;
        require_entry(&tx, &entry)?;
        tx.commit()?;
        Ok(AttemptPermit {
            request: entry.request_id,
            provider: entry.provider_fingerprint,
            quote: entry.estimated_microusd,
        })
    }
    fn cancel_unattempted(&mut self, request_id: &str) -> Result<()> {
        validate_path(&self.path, true)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        validate_connection(&tx, &self.path)?;
        let mut entry = read_entry(&tx, request_id)?.ok_or(Error::Confirmation)?;
        match entry.state {
            EntryState::Reserved => {
                require_changed(tx.execute(
                    "UPDATE entries SET state=3 WHERE request=?1 AND state=0",
                    [request_id],
                )?)?;
            }
            EntryState::NotSent => {}
            _ => return Err(Error::AttemptConsumed),
        }
        entry.state = EntryState::NotSent;
        require_entry(&tx, &entry)?;
        tx.commit()?;
        Ok(())
    }
    fn settle(
        &mut self,
        request_id: &str,
        provider: &ProviderConfig,
        tokens: Tokens,
    ) -> Result<u64> {
        let fingerprint = provider.fingerprint()?;
        if tokens.image_output == 0 {
            return Err(Error::Invalid("settlement usage"));
        }
        let actual = provider.prices.cost(tokens)?;
        let actual_i = i64::try_from(actual).map_err(|_| Error::Limit("ledger charge"))?;
        validate_path(&self.path, true)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        validate_connection(&tx, &self.path)?;
        let mut entry = read_entry(&tx, request_id)?.ok_or(Error::Confirmation)?;
        if entry.provider_fingerprint != fingerprint {
            return Err(Error::Confirmation);
        }
        if entry.state == EntryState::Settled && entry.actual_microusd == Some(actual) {
            tx.commit()?;
            return Ok(actual);
        }
        if entry.state != EntryState::Attempted {
            return Err(Error::AttemptConsumed);
        }
        require_changed(tx.execute(
            "UPDATE entries SET state=2,actual=?2 WHERE request=?1 AND state=1",
            params![request_id, actual_i],
        )?)?;
        entry.state = EntryState::Settled;
        entry.actual_microusd = Some(actual);
        require_entry(&tx, &entry)?;
        tx.commit()?;
        Ok(actual)
    }
    fn entry(&self, request_id: &str) -> Result<Option<LedgerEntry>> {
        validate_path(&self.path, true)?;
        let tx = self.connection.unchecked_transaction()?;
        validate_connection(&tx, &self.path)?;
        let result = read_entry(&tx, request_id)?;
        tx.commit()?;
        Ok(result)
    }
    fn spent_on(&self, utc_day: u32) -> Result<u64> {
        validate_path(&self.path, true)?;
        let tx = self.connection.unchecked_transaction()?;
        validate_connection(&tx, &self.path)?;
        let result = spent_on_connection(&tx, utc_day)?;
        tx.commit()?;
        Ok(result)
    }
}

fn require_changed(changed: usize) -> Result<()> {
    if changed == 1 {
        Ok(())
    } else {
        Err(Error::Storage)
    }
}
fn require_entry(connection: &Connection, expected: &LedgerEntry) -> Result<()> {
    if read_entry(connection, &expected.request_id)?.as_ref() == Some(expected) {
        Ok(())
    } else {
        Err(Error::Storage)
    }
}

fn validate_connection(connection: &Connection, path: &Path) -> Result<()> {
    validate_path(path, true)?;
    validate_page_limits(connection, false)?;
    let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let application: i64 = connection.query_row("PRAGMA application_id", [], |row| row.get(0))?;
    if version != 1 || application != APPLICATION_ID {
        return Err(Error::Storage);
    }
    validate_schema(connection)?;

    // All persisted TEXT is checked inside SQLite before row_entry can allocate
    // a Rust String. The connection's 8 KiB row limit also bounds malformed rows
    // while SQLite evaluates length/typeof or loads its schema.
    let malformed: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM entries WHERE typeof(request)<>'text' OR length(request)<>64 OR length(CAST(request AS BLOB))<>64 OR request GLOB '*[^0-9a-f]*' OR typeof(provider)<>'text' OR length(provider)<>64 OR length(CAST(provider AS BLOB))<>64 OR provider GLOB '*[^0-9a-f]*' OR typeof(utc_day)<>'integer' OR utc_day NOT BETWEEN 1 AND 1000000 OR typeof(quote)<>'integer' OR quote<=0 OR typeof(state)<>'integer' OR state NOT BETWEEN 0 AND 3 OR typeof(sequence)<>'integer' OR sequence<=0 OR (state=2 AND (typeof(actual)<>'integer' OR actual<0)) OR (state<>2 AND typeof(actual)<>'null'))",
        [], |row| row.get(0),
    )?;
    let bad_meta: bool = connection.query_row(
        "SELECT count(*)<>1 OR coalesce(sum(typeof(id)<>'integer' OR id<>1 OR typeof(format)<>'integer' OR format<>1 OR typeof(sequence)<>'integer' OR sequence<0 OR typeof(last_day)<>'integer' OR last_day NOT BETWEEN 0 AND 1000000),0)<>0 FROM meta",
        [], |row| row.get(0),
    )?;
    if malformed || bad_meta {
        return Err(Error::Storage);
    }
    let (sequence, last_day): (i64, i64) =
        connection.query_row("SELECT sequence,last_day FROM meta WHERE id=1", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?;
    let (count, max_sequence, max_day, unresolved): (i64, i64, i64, i64) = connection.query_row(
        "SELECT count(*),coalesce(max(sequence),0),coalesce(max(utc_day),0),coalesce(sum(state IN (0,1)),0) FROM entries",
        [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    if !(0..=MAX_ENTRIES).contains(&count)
        || sequence != count
        || max_sequence != sequence
        || last_day != max_day
        || unresolved > 1
    {
        return Err(Error::Storage);
    }
    Ok(())
}

fn validate_schema(connection: &Connection) -> Result<()> {
    let count: i64 =
        connection.query_row("SELECT count(*) FROM sqlite_schema", [], |row| row.get(0))?;
    if count != 4 {
        return Err(Error::Storage);
    }
    let mut statement = connection
        .prepare("SELECT type,name,tbl_name,rootpage,sql FROM sqlite_schema ORDER BY type,name")?;
    let mut rows = statement.query([])?;
    let expected = [
        ("index", "sqlite_autoindex_entries_1", "entries", None),
        ("index", "sqlite_autoindex_entries_2", "entries", None),
        ("table", "entries", "entries", Some(ENTRIES_SCHEMA)),
        ("table", "meta", "meta", Some(META_SCHEMA)),
    ];
    for (kind, name, table, sql) in expected {
        let row = rows.next()?.ok_or(Error::Storage)?;
        let actual_kind: String = row.get(0)?;
        let actual_name: String = row.get(1)?;
        let actual_table: String = row.get(2)?;
        let root: i64 = row.get(3)?;
        let actual_sql: Option<String> = row.get(4)?;
        if actual_kind != kind
            || actual_name != name
            || actual_table != table
            || actual_sql.as_deref() != sql
            || !(1..=MAX_PAGES).contains(&root)
        {
            return Err(Error::Storage);
        }
    }
    if rows.next()?.is_some() {
        return Err(Error::Storage);
    }
    Ok(())
}

type EntryRow = (String, String, i64, i64, i64, Option<i64>);
fn row_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<EntryRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
    ))
}
fn validate_entry(row: &EntryRow) -> Result<LedgerEntry> {
    let (request, provider, day, quote, state, actual) = row;
    if !valid_hash(request)
        || !valid_hash(provider)
        || !(1..=1_000_000).contains(day)
        || *quote <= 0
    {
        return Err(Error::Storage);
    }
    let state = match state {
        0 => EntryState::Reserved,
        1 => EntryState::Attempted,
        2 => EntryState::Settled,
        3 => EntryState::NotSent,
        _ => return Err(Error::Storage),
    };
    if (state == EntryState::Settled) != actual.is_some() || actual.is_some_and(|n| n < 0) {
        return Err(Error::Storage);
    }
    Ok(LedgerEntry {
        request_id: request.clone(),
        provider_fingerprint: provider.clone(),
        utc_day: *day as u32,
        estimated_microusd: *quote as u64,
        actual_microusd: actual.map(|n| n as u64),
        state,
    })
}
fn read_entry(connection: &Connection, request_id: &str) -> Result<Option<LedgerEntry>> {
    if !valid_hash(request_id) {
        return Err(Error::Invalid("request ID"));
    }
    connection
        .query_row(
            "SELECT request,provider,utc_day,quote,state,actual FROM entries WHERE request=?1",
            [request_id],
            row_entry,
        )
        .optional()?
        .as_ref()
        .map(validate_entry)
        .transpose()
}
fn spent_on_connection(connection: &Connection, day: u32) -> Result<u64> {
    let mut statement =
        connection.prepare("SELECT quote,state,actual FROM entries WHERE utc_day=?1")?;
    let mut rows = statement.query([day])?;
    let mut total = 0u64;
    while let Some(row) = rows.next()? {
        let quote: i64 = row.get(0)?;
        let state: i64 = row.get(1)?;
        let actual: Option<i64> = row.get(2)?;
        let charge = match state {
            0 | 1 => quote,
            2 => actual.ok_or(Error::Storage)?,
            3 => 0,
            _ => return Err(Error::Storage),
        };
        total = total
            .checked_add(u64::try_from(charge).map_err(|_| Error::Storage)?)
            .ok_or(Error::Limit("daily charge"))?;
    }
    Ok(total)
}
fn connect(path: &Path, creating: bool) -> Result<Connection> {
    if !creating {
        // Reject incompatible page sizes before a SQLite connection can allocate
        // page buffers or parse a malicious schema. WAL recovery remains enabled.
        let mut header = [0u8; 100];
        std::fs::File::open(path)?.read_exact(&mut header)?;
        if &header[..16] != b"SQLite format 3\0"
            || u16::from_be_bytes([header[16], header[17]]) != PAGE_BYTES as u16
        {
            return Err(Error::Storage);
        }
    }
    let connection = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    // These apply before the first schema-reading statement, including PRAGMAs.
    connection.set_limit(Limit::SQLITE_LIMIT_LENGTH, MAX_SQLITE_VALUE_BYTES)?;
    connection.set_limit(Limit::SQLITE_LIMIT_SQL_LENGTH, MAX_SQLITE_VALUE_BYTES)?;
    connection.set_limit(Limit::SQLITE_LIMIT_ATTACHED, 0)?;
    connection.set_limit(Limit::SQLITE_LIMIT_COLUMN, 16)?;
    connection.set_limit(Limit::SQLITE_LIMIT_VARIABLE_NUMBER, 32)?;
    connection.set_limit(Limit::SQLITE_LIMIT_TRIGGER_DEPTH, 0)?;
    connection.set_limit(Limit::SQLITE_LIMIT_WORKER_THREADS, 0)?;
    connection.busy_timeout(Duration::from_millis(500))?;
    connection.execute_batch("PRAGMA trusted_schema=OFF; PRAGMA page_size=4096; PRAGMA cache_size=-256; PRAGMA mmap_size=0;")?;
    validate_page_limits(&connection, creating)?;
    let maximum: i64 = connection.query_row("PRAGMA max_page_count=2048", [], |row| row.get(0))?;
    if maximum != MAX_PAGES {
        return Err(Error::Storage);
    }
    let journal: String = connection.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))?;
    if journal != "wal" {
        return Err(Error::Storage);
    }
    connection.execute_batch("PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; PRAGMA journal_size_limit=1048576; PRAGMA wal_autocheckpoint=64;")?;
    Ok(connection)
}
fn validate_page_limits(connection: &Connection, allow_empty: bool) -> Result<()> {
    let page_size: i64 = connection.query_row("PRAGMA page_size", [], |row| row.get(0))?;
    let page_count: i64 = connection.query_row("PRAGMA page_count", [], |row| row.get(0))?;
    if page_size != PAGE_BYTES as i64
        || page_count < i64::from(!allow_empty)
        || page_count > MAX_PAGES
    {
        return Err(Error::Storage);
    }
    Ok(())
}
fn validate_path(path: &Path, existing: bool) -> Result<()> {
    if !path.is_absolute() || path.file_name().is_none() {
        return Err(Error::Storage);
    }
    let mut current = Some(path);
    let mut first = true;
    while let Some(value) = current {
        match fs::symlink_metadata(value) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(Error::Storage);
                }
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if metadata.file_attributes() & 0x400 != 0 {
                        return Err(Error::Storage);
                    }
                }
                if (first && !metadata.is_file()) || (!first && !metadata.is_dir()) {
                    return Err(Error::Storage);
                }
                if first
                    && (metadata.len() > MAX_LEDGER_BYTES
                        || existing && metadata.len() < PAGE_BYTES)
                {
                    return Err(Error::Storage);
                }
            }
            Err(e) if first && !existing && e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(Error::Storage),
        }
        first = false;
        current = value.parent();
    }
    // SQLite sidecar names must not redirect writes outside the private root.
    for (suffix, maximum) in [
        ("-wal", MAX_LEDGER_WAL_BYTES),
        ("-shm", MAX_SHM_BYTES),
        ("-journal", 2 * MAX_LEDGER_BYTES),
    ] {
        let mut name = path.as_os_str().to_os_string();
        name.push(suffix);
        match fs::symlink_metadata(Path::new(&name)) {
            Ok(metadata) => {
                if !existing
                    || !metadata.is_file()
                    || metadata.file_type().is_symlink()
                    || metadata.len() > maximum
                {
                    return Err(Error::Storage);
                }
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if metadata.file_attributes() & 0x400 != 0 {
                        return Err(Error::Storage);
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(Error::Storage),
        }
    }
    Ok(())
}
