mod common;
use common::*;
use rusqlite::{Connection, params};
use vw_ai::{budget::*, config::Tokens, *};

const RESET_ATTEMPT: &str = "CREATE TRIGGER sqlitex AFTER UPDATE OF state ON entries WHEN NEW.state=1 BEGIN UPDATE entries SET state=0 WHERE request=NEW.request; END";
const IGNORE_TRANSITION: &str =
    "CREATE TRIGGER sqlitex BEFORE UPDATE OF state ON entries BEGIN SELECT RAISE(IGNORE); END";

#[test]
fn trigger_cannot_issue_a_permit_before_reopen_or_after_open() -> TestResult {
    for sql in [RESET_ATTEMPT, IGNORE_TRANSITION] {
        for reopen in [false, true] {
            let root = temp()?;
            let path = root.path().join("ledger.sqlite");
            let prepared = prepared(1)?;
            let mut ledger = SqliteBudgetLedger::create(&path)?;
            ledger.reserve(
                prepared.review(),
                Confirmation::explicit_send(prepared.review(), 20000, false)?,
                BudgetPolicy::default(),
            )?;
            let connection = Connection::open(&path)?;
            connection.execute_batch(sql)?;
            if reopen {
                drop(ledger);
                assert!(matches!(
                    SqliteBudgetLedger::open(&path),
                    Err(Error::Storage)
                ));
            } else {
                assert!(matches!(
                    ledger.begin_attempt(prepared.review()),
                    Err(Error::Storage)
                ));
                assert!(matches!(
                    ledger.begin_attempt(prepared.review()),
                    Err(Error::Storage)
                ));
                assert_eq!(
                    ledger.cancel_unattempted(prepared.review().request_id()),
                    Err(Error::Storage)
                );
                drop(ledger);
            }
            assert_eq!(
                connection
                    .query_row("SELECT state FROM entries", [], |row| row.get::<_, i64>(0))?,
                0
            );
            connection.execute_batch("DROP TRIGGER sqlitex")?;
            drop(connection);
            let mut reopened = SqliteBudgetLedger::open(&path)?;
            drop(reopened.begin_attempt(prepared.review())?);
            assert!(matches!(
                reopened.begin_attempt(prepared.review()),
                Err(Error::AttemptConsumed)
            ));
        }
    }
    Ok(())
}

#[test]
fn settlement_and_fresh_reservations_recheck_schema_after_open() -> TestResult {
    let root = temp()?;
    let path = root.path().join("ledger.sqlite");
    let prepared = prepared(1)?;
    let other = common::prepared(2)?;
    let mut ledger = SqliteBudgetLedger::create(&path)?;
    ledger.reserve(
        prepared.review(),
        Confirmation::explicit_send(prepared.review(), 20000, false)?,
        BudgetPolicy::default(),
    )?;
    drop(ledger.begin_attempt(prepared.review())?);
    let connection = Connection::open(&path)?;
    connection.execute_batch(IGNORE_TRANSITION)?;
    let tokens = Tokens {
        text_input: 1,
        image_input: 2,
        image_output: 3,
    };
    assert_eq!(
        ledger.settle(prepared.review().request_id(), prepared.provider(), tokens),
        Err(Error::Storage)
    );
    assert_eq!(
        connection.query_row("SELECT state FROM entries", [], |row| row.get::<_, i64>(0))?,
        1
    );
    connection.execute_batch("DROP TRIGGER sqlitex")?;
    let charge = ledger.settle(prepared.review().request_id(), prepared.provider(), tokens)?;
    drop(ledger);
    let mut ledger = SqliteBudgetLedger::open(&path)?;
    assert_eq!(
        ledger.settle(prepared.review().request_id(), prepared.provider(), tokens)?,
        charge
    );
    connection.execute_batch("CREATE INDEX sqlitex ON entries(utc_day)")?;
    assert_eq!(
        ledger.reserve(
            other.review(),
            Confirmation::explicit_send(other.review(), 20001, true)?,
            BudgetPolicy::default()
        ),
        Err(Error::Storage)
    );
    assert_eq!(
        connection.query_row("SELECT count(*) FROM entries", [], |row| row
            .get::<_, i64>(0))?,
        1
    );
    connection.execute_batch("DROP INDEX sqlitex")?;
    ledger.reserve(
        other.review(),
        Confirmation::explicit_send(other.review(), 20001, true)?,
        BudgetPolicy::default(),
    )?;
    Ok(())
}

#[test]
fn oversized_persisted_text_fails_before_returning_a_string_or_permit() -> TestResult {
    for field in ["provider", "request"] {
        let root = temp()?;
        let path = root.path().join("ledger.sqlite");
        let prepared = prepared(1)?;
        let mut ledger = SqliteBudgetLedger::create(&path)?;
        ledger.reserve(
            prepared.review(),
            Confirmation::explicit_send(prepared.review(), 20000, false)?,
            BudgetPolicy::default(),
        )?;
        let connection = Connection::open(&path)?;
        // The SQL creates a 1 MiB TEXT inside SQLite. Rust never builds that
        // String, and admission's 8 KiB row limit rejects it before row_entry.
        connection.execute_batch(match field {
            "provider" => "UPDATE entries SET provider=hex(zeroblob(524288))",
            _ => "UPDATE entries SET request=hex(zeroblob(524288))",
        })?;
        assert!(matches!(
            ledger.begin_attempt(prepared.review()),
            Err(Error::Storage)
        ));
        assert_eq!(
            ledger.entry(prepared.review().request_id()),
            Err(Error::Storage)
        );
        drop(ledger);
        assert!(matches!(
            SqliteBudgetLedger::open(&path),
            Err(Error::Storage)
        ));
        assert_eq!(
            connection.query_row("SELECT state FROM entries", [], |row| row.get::<_, i64>(0))?,
            0
        );
    }
    Ok(())
}

#[test]
fn sqlite_page_limit_rejects_existing_growth_in_wal_and_database() -> TestResult {
    let root = temp()?;
    let path = root.path().join("ledger.sqlite");
    drop(SqliteBudgetLedger::create(&path)?);
    let writer = Connection::open(&path)?;
    writer.execute_batch("PRAGMA wal_autocheckpoint=0; PRAGMA max_page_count=4096;")?;
    let reader = Connection::open(&path)?;
    reader.execute_batch("BEGIN; SELECT count(*) FROM entries;")?;
    // Keep only the original schema, but leave >2048 pages on the freelist.
    // They initially live in WAL while the main file is still within its cap.
    writer.execute_batch("CREATE TABLE padding(bytes BLOB); INSERT INTO padding VALUES(zeroblob(9437184)); DROP TABLE padding;")?;
    assert!(std::fs::metadata(&path)?.len() <= MAX_LEDGER_BYTES);
    assert!(writer.query_row("PRAGMA page_count", [], |row| row.get::<_, i64>(0))? > 2048);
    assert!(matches!(
        SqliteBudgetLedger::open(&path),
        Err(Error::Storage)
    ));
    reader.execute_batch("ROLLBACK")?;
    writer.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
    drop(reader);
    drop(writer);
    let length = std::fs::metadata(&path)?.len();
    assert!(length > MAX_LEDGER_BYTES);
    assert!(matches!(
        SqliteBudgetLedger::open(&path),
        Err(Error::Storage)
    ));
    assert_eq!(std::fs::metadata(&path)?.len(), length);
    Ok(())
}

#[test]
fn over_limit_sidecar_lengths_are_refused_without_opening_or_deleting_them() -> TestResult {
    for (suffix, maximum) in [
        ("-wal", MAX_LEDGER_WAL_BYTES),
        ("-shm", 16 * 1024 * 1024),
        ("-journal", 2 * MAX_LEDGER_BYTES),
    ] {
        let root = temp()?;
        let path = root.path().join("ledger.sqlite");
        drop(SqliteBudgetLedger::create(&path)?);
        let original = std::fs::read(&path)?;
        let mut name = path.as_os_str().to_os_string();
        name.push(suffix);
        let sidecar = std::path::PathBuf::from(name);
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&sidecar)?;
        // Windows allocates extended ordinary files on disk. Mark this owned
        // length-only fixture sparse before extending it; the admission oracle
        // still sees the actual over-limit metadata, with no gigabyte allocation.
        #[cfg(windows)]
        sparse_fixture::mark(&file)?;
        file.set_len(maximum + 1)?;
        drop(file);
        assert!(matches!(
            SqliteBudgetLedger::open(&path),
            Err(Error::Storage)
        ));
        assert_eq!(std::fs::metadata(&sidecar)?.len(), maximum + 1);
        assert_eq!(std::fs::read(&path)?, original);
    }
    Ok(())
}

#[cfg(windows)]
mod sparse_fixture {
    use std::{ffi::c_void, fs::File, io, os::windows::io::AsRawHandle};

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn DeviceIoControl(
            file: *mut c_void,
            code: u32,
            input: *mut c_void,
            input_bytes: u32,
            output: *mut c_void,
            output_bytes: u32,
            returned: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
    }

    pub(super) fn mark(file: &File) -> io::Result<()> {
        const FSCTL_SET_SPARSE: u32 = 0x0009_00c4;
        let mut returned = 0;
        // SAFETY: File owns a valid synchronous handle throughout this call.
        // Null input sets sparse=true; this IOCTL has no output buffer. The
        // writable byte-count pointer lives until the synchronous call returns.
        let success = unsafe {
            DeviceIoControl(
                file.as_raw_handle(),
                FSCTL_SET_SPARSE,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        if success == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

#[test]
fn full_history_with_nonempty_wal_remains_readable_and_recoverable() -> TestResult {
    let root = temp()?;
    let path = root.path().join("ledger.sqlite");
    drop(SqliteBudgetLedger::create(&path)?);
    let mut writer = Connection::open(&path)?;
    writer.execute_batch("PRAGMA wal_autocheckpoint=0")?;
    let reader = Connection::open(&path)?;
    reader.execute_batch("BEGIN; SELECT count(*) FROM entries;")?;
    {
        let tx = writer.transaction()?;
        for number in 1..=4096 {
            tx.execute(
                "INSERT INTO entries VALUES(?1,?2,20000,140,3,NULL,?3)",
                params![format!("{number:064x}"), "b".repeat(64), number],
            )?;
        }
        tx.execute(
            "UPDATE meta SET sequence=4096,last_day=20000 WHERE id=1",
            [],
        )?;
        tx.commit()?;
    }
    let mut wal_name = path.as_os_str().to_os_string();
    wal_name.push("-wal");
    assert!(std::fs::metadata(std::path::Path::new(&wal_name))?.len() > 32);
    let mut ledger = SqliteBudgetLedger::open(&path)?;
    assert_eq!(ledger.spent_on(20000)?, 0);
    assert_eq!(
        ledger
            .entry(&format!("{:064x}", 4096))?
            .ok_or("last entry")?
            .state,
        EntryState::NotSent
    );
    let prepared = prepared(1)?;
    assert!(matches!(
        ledger.reserve(
            prepared.review(),
            Confirmation::explicit_send(prepared.review(), 20001, true)?,
            BudgetPolicy::default()
        ),
        Err(Error::Limit(_))
    ));
    drop(ledger);
    reader.execute_batch("ROLLBACK")?;
    drop(reader);
    drop(writer);
    assert_eq!(SqliteBudgetLedger::open(&path)?.spent_on(20000)?, 0);
    Ok(())
}
