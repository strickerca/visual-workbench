use crate::{
    StoreError,
    database::{decode, replay_log, write_conflict, write_snapshot},
};
use rusqlite::{Connection, TransactionBehavior};
use vw_model::{DeviceId, Project};
use vw_ops::HostSequencer;

pub(crate) const SCHEMA: &str = include_str!("../../../../contracts/storage.sql");
const MIGRATION_2: &str = include_str!("../migrations/002_checkpoints.sql");
const MIGRATION_1: &str = include_str!("../migrations/001_draft.sql");
type SchemaRows = Vec<(String, String, String, String)>;
static SCHEMA_1: std::sync::OnceLock<SchemaRows> = std::sync::OnceLock::new();
static SCHEMA_2: std::sync::OnceLock<SchemaRows> = std::sync::OnceLock::new();

pub(crate) fn verify_schema(connection: &Connection, version: i64) -> Result<(), StoreError> {
    let (sql, cached) = match version {
        1 => (MIGRATION_1, &SCHEMA_1),
        2 => (SCHEMA, &SCHEMA_2),
        other => return Err(StoreError::UnsupportedSchema(other)),
    };
    if cached.get().is_none() {
        let reference = Connection::open_in_memory()?;
        reference.execute_batch(sql)?;
        let _ = cached.set(schema_rows(&reference)?);
    }
    if Some(&schema_rows(connection)?) != cached.get() {
        return Err(StoreError::Corrupt("unexpected database schema"));
    }
    Ok(())
}
fn schema_rows(connection: &Connection) -> Result<SchemaRows, StoreError> {
    let excessive:i64=connection.query_row("SELECT COUNT(*) FROM sqlite_master WHERE length(name)>256 OR length(tbl_name)>256 OR length(sql)>262144",[],|r|r.get(0))?;
    let count: i64 =
        connection.query_row("SELECT COUNT(*) FROM sqlite_master", [], |r| r.get(0))?;
    if excessive != 0 || count > 256 {
        return Err(StoreError::Corrupt("database schema bounds"));
    }
    Ok(connection.prepare("SELECT type,name,tbl_name,COALESCE(sql,'') FROM sqlite_master WHERE name NOT GLOB 'sqlite_*' ORDER BY type,name")?.query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?.collect::<Result<Vec<_>,_>>()?)
}

pub(crate) fn upgrade(connection: &mut Connection) -> Result<(), StoreError> {
    if connection.query_row(
        "SELECT length(CAST(value AS BLOB)) FROM meta WHERE key='schema_version'",
        [],
        |r| r.get::<_, i64>(0),
    )? > 16
    {
        return Err(StoreError::Corrupt("schema version"));
    }
    let version: String = connection.query_row(
        "SELECT value FROM meta WHERE key='schema_version'",
        [],
        |r| r.get(0),
    )?;
    let version: i64 = version
        .parse()
        .map_err(|_| StoreError::Corrupt("schema version"))?;
    verify_schema(connection, version)?;
    match version {
        2 => return Ok(()),
        1 => {}
        other => return Err(StoreError::UnsupportedSchema(other)),
    }
    // Draft 1 was not a shipped storage implementation. Its supported synthetic
    // fixtures explicitly include revision zero and the original host identity;
    // inferring either from a later snapshot would destroy retry/undo semantics.
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    crate::database::check_all_column_bounds(&tx)?;
    for sql in [
        "SELECT COUNT(*) FROM snapshots WHERE length(state)>67108864",
        "SELECT COUNT(*) FROM op_log WHERE length(txn)>67108864",
        "SELECT COUNT(*) FROM pending_txns WHERE length(txn)>67108864",
    ] {
        if tx.query_row(sql, [], |r| r.get::<_, i64>(0))? != 0 {
            return Err(StoreError::Corrupt("migration payload size"));
        }
    }
    let device: String =
        tx.query_row("SELECT value FROM meta WHERE key='host_device'", [], |r| {
            r.get(0)
        })?;
    let (state, hash, time): (Vec<u8>, String, i64) = tx.query_row(
        "SELECT state,state_hash,created_at FROM snapshots WHERE host_seq=0",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let project = Project::from_canonical_bytes(&state)?;
    if project.state_hash()?.as_str() != hash || time < 0 {
        return Err(StoreError::Corrupt("migration bootstrap"));
    }
    let project_id: String =
        tx.query_row("SELECT value FROM meta WHERE key='project_id'", [], |r| {
            r.get(0)
        })?;
    if project_id != project.id.as_str() {
        return Err(StoreError::Corrupt("migration project identity"));
    }
    let mut host = HostSequencer::new(project, DeviceId::try_from(device)?)?;
    tx.execute_batch(MIGRATION_2)?;
    crate::projections::persist(&tx, host.project(), 0, time)?;
    write_snapshot(&tx, &host, time)?;
    replay_log(&tx, &mut host, 0, false)?;
    // Verify every pre-existing snapshot against independent historical replay.
    let snapshots = tx
        .prepare("SELECT host_seq,state,state_hash FROM snapshots ORDER BY host_seq")?
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Vec<u8>>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (seq, bytes, hash) in snapshots {
        let state = Project::from_canonical_bytes(&bytes)?;
        if state.state_hash()?.as_str() != hash {
            return Err(StoreError::Corrupt("migration snapshot"));
        }
        if seq > 0 {
            let expected: String = tx.query_row(
                "SELECT state_hash FROM op_log WHERE host_seq=?1",
                [seq],
                |r| r.get(0),
            )?;
            if expected != hash {
                return Err(StoreError::Corrupt("migration snapshot log binding"));
            }
        }
    }
    // Complete immutable historical asset metadata, even if later undone.
    let transactions = tx
        .prepare("SELECT txn FROM op_log ORDER BY host_seq")?
        .query_map([], |r| r.get::<_, Vec<u8>>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    for bytes in transactions {
        let txn: vw_proto::v1::Transaction = decode(&bytes)?;
        for op in txn.ops {
            if let Some(vw_proto::v1::op::Kind::AddAsset(asset)) = op.kind {
                crate::projections::persist_asset(&tx, &asset)?;
            }
        }
    }
    crate::projections::persist(
        &tx,
        host.project(),
        i64::try_from(host.revision()?.host_seq)
            .map_err(|_| StoreError::Invalid("sequence range"))?,
        time,
    )?;
    tx.execute("DELETE FROM conflicts", [])?;
    for record in host.conflicts() {
        write_conflict(&tx, record)?;
    }
    // Keep revision zero plus the fully restored latest checkpoint. Old draft
    // snapshots with no sequencing state cannot safely act as restore points.
    tx.execute("DELETE FROM snapshots WHERE host_seq>0", [])?;
    write_snapshot(&tx, &host, time)?;
    for txn in crate::database::validate_pending(&tx, host.project())? {
        for op in txn.ops {
            if let Some(vw_proto::v1::op::Kind::AddAsset(asset)) = op.kind {
                crate::projections::persist_asset(&tx, &asset)?;
            }
        }
    }
    crate::projections::verify_assets(&tx, host.project())?;
    crate::database::validate_pending(&tx, host.project())?;
    crate::database::integrity(&tx)?;
    tx.commit()?;
    Ok(())
}
