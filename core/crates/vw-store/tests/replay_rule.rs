//! The replay rule (stated at `evaluate` in vw-ops): a project opens by
//! deciding its stored transactions again, so what this build decides for a
//! transaction an earlier build accepted has to stay the same. These tests
//! pin that for every path the engine can take with two devices, replay logs
//! an older build stored, and show what an open does when it does not hold.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "replay_rule/script.rs"]
mod script;

use script::{CANCELLED, CATCHES_UP, CHECKPOINTS, JOINS, device, id, run, steps};
use std::{
    fs,
    path::{Path, PathBuf},
};
use tempfile::TempDir;
use vw_proto::{Message, v1 as pb};
use vw_store::{ProjectStore, StoreError};

/// The script as a build from before 2026-11-14 stored it, when the engine
/// was rewritten to decide in place: the host's project and the other
/// device's. See `fixtures/README.md`.
const STORED_HOST: &[u8] = include_bytes!("fixtures/replay-host.sqlite");
const STORED_MIRROR: &[u8] = include_bytes!("fixtures/replay-mirror.sqlite");

/// What every build has to decide for the script: each stored transaction
/// with the revision and state hash it was accepted at, and every conflict
/// record. Notices are not part of it.
const PINNED_LOG: &str = "43f9ddacf471f1e9caf117e14d09160eeea82dedc7eb59c1ac637adc353c491f";
/// The sequencing state the script ends in: the project, the journal every
/// undo depends on, who wrote what last, tombstones and conflicts, as a
/// checkpoint writes them with the notices taken out.
const PINNED_STATE: &str = "dc323c4a96f9519b5f618ec631b3fa3e2f6ad32a5348a078a5e220d7d22da9a2";

fn temporary() -> TempDir {
    let mut builder = tempfile::Builder::new();
    builder.prefix("vw-replay-rule-");
    #[cfg(target_os = "android")]
    {
        builder
            .tempdir_in(std::env::current_dir().unwrap())
            .unwrap()
    }
    #[cfg(not(target_os = "android"))]
    {
        builder.tempdir().unwrap()
    }
}
fn query(path: &Path) -> rusqlite::Connection {
    rusqlite::Connection::open(path.join("project.sqlite")).unwrap()
}
fn checkpoint_now(store: &mut ProjectStore, now_ms: i64) {
    store.checkpoint_now(now_ms).unwrap();
}

struct LogRow {
    seq: i64,
    txn: Vec<u8>,
    state_hash: String,
    accepted_at: i64,
    ack: Vec<u8>,
}
fn log(path: &Path) -> Vec<LogRow> {
    query(path)
        .prepare("SELECT host_seq,txn,state_hash,accepted_at,ack FROM op_log ORDER BY host_seq")
        .unwrap()
        .query_map([], |row| {
            Ok(LogRow {
                seq: row.get(0)?,
                txn: row.get(1)?,
                state_hash: row.get(2)?,
                accepted_at: row.get(3)?,
                ack: row.get(4)?,
            })
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}
fn conflicts(path: &Path) -> Vec<Vec<u8>> {
    query(path)
        .prepare("SELECT record FROM conflicts ORDER BY rowid")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}
fn part(hasher: &mut blake3::Hasher, bytes: &[u8]) {
    hasher.update(&(bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

/// The digest `PINNED_LOG` pins, from the rows a project stores.
fn outcome(path: &Path) -> String {
    let mut hasher = blake3::Hasher::new();
    for row in log(path) {
        let ack = pb::TxnAck::decode(row.ack.as_slice()).unwrap();
        assert_eq!(ack.host_seq, row.seq as u64);
        assert_eq!(
            blake3::Hash::from_bytes(ack.state_hash.as_slice().try_into().unwrap())
                .to_hex()
                .as_str(),
            row.state_hash
        );
        hasher.update(&row.seq.to_le_bytes());
        hasher.update(&row.accepted_at.to_le_bytes());
        part(&mut hasher, &row.txn);
        part(&mut hasher, row.state_hash.as_bytes());
    }
    for record in conflicts(path) {
        part(&mut hasher, &record);
    }
    hasher.finalize().to_hex().to_string()
}

/// The digest `PINNED_STATE` pins, from the sequencing state a store holds.
fn state(checkpoint: &[u8]) -> String {
    const PREFIX: &[u8] = b"VisualWorkbench.HostCheckpoint.v1\n";
    let payload = &checkpoint.strip_prefix(PREFIX).unwrap()[32..];
    let mut value: serde_json::Value = serde_json::from_slice(payload).unwrap();
    let records = value["records"].as_object_mut().unwrap();
    assert!(!records.is_empty());
    for record in records.values_mut() {
        record["ack"]["notices"] = serde_json::Value::Array(Vec::new());
    }
    blake3::hash(&serde_json::to_vec(&value).unwrap())
        .to_hex()
        .to_string()
}

/// The stored notices of every acknowledgement that has any, by revision.
fn notices(path: &Path) -> Vec<(i64, Vec<String>)> {
    log(path)
        .into_iter()
        .map(|row| {
            (
                row.seq,
                pb::TxnAck::decode(row.ack.as_slice()).unwrap().notices,
            )
        })
        .filter(|(_, notices)| !notices.is_empty())
        .collect()
}

/// The seal of the log up to a revision as the store computes it. A test
/// that changes stored rows the way another build would have written them
/// seals them again.
fn seal_through(path: &Path, through: i64) -> String {
    let mut digest = blake3::hash(b"VisualWorkbench.LogSeal.v1\n");
    let mut last = 0;
    for row in log(path).into_iter().filter(|row| row.seq <= through) {
        let mut hasher = blake3::Hasher::new();
        hasher.update(digest.as_bytes());
        hasher.update(&row.seq.to_le_bytes());
        hasher.update(&row.accepted_at.to_le_bytes());
        for bytes in [&row.txn[..], row.state_hash.as_bytes(), &row.ack[..]] {
            part(&mut hasher, bytes);
        }
        digest = hasher.finalize();
        last = row.seq;
    }
    format!("v1 {last} {}", digest.to_hex())
}
fn seal(path: &Path) -> String {
    seal_through(path, i64::MAX)
}
fn stored_seal(path: &Path) -> Option<String> {
    use rusqlite::OptionalExtension;
    query(path)
        .query_row("SELECT value FROM meta WHERE key='log_seal'", [], |row| {
            row.get(0)
        })
        .optional()
        .unwrap()
}
fn set_seal(path: &Path, seal: &str) {
    assert_eq!(
        query(path)
            .execute("UPDATE meta SET value=?1 WHERE key='log_seal'", [seal])
            .unwrap(),
        1
    );
}
fn reseal(path: &Path) {
    set_seal(path, &seal(path));
}
fn checkpoints(path: &Path) -> Vec<i64> {
    query(path)
        .prepare("SELECT host_seq FROM snapshots ORDER BY host_seq")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// Everything an open could have changed.
fn stored(path: &Path) -> Vec<(String, Vec<u8>)> {
    let database = query(path);
    let mut all = Vec::new();
    for sql in [
        "SELECT 'checkpoint ' || host_seq, checkpoint FROM snapshots ORDER BY host_seq",
        "SELECT 'state ' || host_seq, state FROM snapshots ORDER BY host_seq",
        "SELECT 'txn ' || host_seq, txn FROM op_log ORDER BY host_seq",
        "SELECT 'ack ' || host_seq || ' ' || state_hash, ack FROM op_log ORDER BY host_seq",
        "SELECT 'meta ' || key, CAST(value AS BLOB) FROM meta ORDER BY key",
        "SELECT 'conflict ' || conflict_id, record FROM conflicts ORDER BY rowid",
    ] {
        let mut query = database.prepare(sql).unwrap();
        let rows = query
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap();
        all.extend(rows.map(Result::unwrap));
    }
    all
}
/// Run the script with this build. Returns the host's project, the other
/// device's, and the sequencing state both ended in.
fn run_script(temp: &TempDir) -> (PathBuf, PathBuf, Vec<u8>) {
    let (host, mirror) = run(temp.path(), checkpoint_now);
    let state = host.checkpoint_bytes().unwrap();
    assert!(mirror.checkpoint_bytes().unwrap() == state);
    drop(host);
    drop(mirror);
    (
        temp.path().join("host.vwb"),
        temp.path().join("mirror.vwb"),
        state,
    )
}
fn rewrite_ack(path: &Path, seq: i64, change: impl FnOnce(&mut pb::TxnAck)) {
    let database = query(path);
    let bytes: Vec<u8> = database
        .query_row("SELECT ack FROM op_log WHERE host_seq=?1", [seq], |row| {
            row.get(0)
        })
        .unwrap();
    let mut ack = pb::TxnAck::decode(bytes.as_slice()).unwrap();
    change(&mut ack);
    database
        .execute(
            "UPDATE op_log SET ack=?1 WHERE host_seq=?2",
            rusqlite::params![ack.encode_to_vec(), seq],
        )
        .unwrap();
}
/// Store the conflict record that matches `filter` with its two devices
/// exchanged: another description of the same resulting project.
fn exchange_conflict_devices(path: &Path, filter: &str) {
    let database = query(path);
    let (key, bytes): (String, Vec<u8>) = database
        .query_row(
            &format!("SELECT conflict_id,record FROM conflicts {filter}"),
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let mut record = pb::ConflictRecord::decode(bytes.as_slice()).unwrap();
    std::mem::swap(&mut record.kept_device, &mut record.other_device);
    database
        .execute(
            "UPDATE conflicts SET kept_device=?1,other_device=?2,record=?3 WHERE conflict_id=?4",
            rusqlite::params![
                record.kept_device,
                record.other_device,
                record.encode_to_vec(),
                key
            ],
        )
        .unwrap();
}

#[test]
fn every_path_two_devices_can_take_is_decided_as_pinned() {
    let temp = temporary();
    let (host, mirror) = run(temp.path(), checkpoint_now);
    let (host_path, mirror_path) = (temp.path().join("host.vwb"), temp.path().join("mirror.vwb"));
    let last = steps().len() as u64;
    assert_eq!(host.revision().unwrap().host_seq, last);

    // The script reaches what it is there for: a conflict each device wins,
    // an edit of a deleted object and a deletion of an edited one, an undo
    // that leaves what the other device changed, a cancellation.
    let records = host.conflicts().unwrap();
    assert_eq!(records.len(), 8);
    for winner in [1, 2] {
        assert!(
            records
                .iter()
                .any(|record| record.kept_device == device(winner).as_str())
        );
    }
    assert_eq!(
        records
            .iter()
            .filter(|record| record.delete_vs_edit && record.edited_object.is_some())
            .count(),
        3
    );
    assert!(
        records
            .iter()
            .any(|record| record.property.starts_with("layers."))
    );
    for store in [&host, &mirror] {
        assert!(store.gesture_closed(&device(2), &id(CANCELLED)));
        assert!(store.gesture_closed(&device(2), &id(40)));
    }
    let state_bytes = host.checkpoint_bytes().unwrap();
    assert!(mirror.checkpoint_bytes().unwrap() == state_bytes);
    drop(host);
    drop(mirror);
    let noticed = notices(&host_path);
    assert_eq!(
        noticed
            .iter()
            .map(|(seq, notices)| (*seq, notices.len()))
            .collect::<Vec<_>>(),
        [(16, 1), (28, 1), (30, 1)]
    );
    assert_eq!(notices(&mirror_path), noticed);

    // What it was decided as. A different value here means this build
    // decides a transaction differently from the builds before it, and their
    // projects stop opening: change the engine back, not these constants.
    assert_eq!(outcome(&host_path), PINNED_LOG);
    assert_eq!(outcome(&mirror_path), PINNED_LOG);
    assert_eq!(state(&state_bytes), PINNED_STATE);
    // The seal is over exactly the stored rows, in a form that is pinned too.
    for path in [&host_path, &mirror_path] {
        assert_eq!(stored_seal(path).unwrap(), seal(path));
    }

    // Opening decides the stored transactions again: the host those since
    // its checkpoint, the other device everything since it joined, and with
    // the later checkpoint gone the host everything there is.
    assert_eq!(checkpoints(&host_path), [0, CHECKPOINTS[1] as i64]);
    assert_eq!(checkpoints(&mirror_path), [JOINS as i64]);
    assert_eq!(CATCHES_UP[2], last);
    for (path, replayed) in [
        (&host_path, last - CHECKPOINTS[1]),
        (&mirror_path, last - JOINS),
        (&host_path, last),
    ] {
        if replayed == last {
            assert_eq!(
                query(path)
                    .execute("DELETE FROM snapshots WHERE host_seq>0", [])
                    .unwrap(),
                1
            );
        }
        let store = ProjectStore::open(path).unwrap();
        assert_eq!(store.replayed_transactions(), replayed);
        assert!(store.checkpoint_bytes().unwrap() == state_bytes);
        assert_eq!(store.conflicts().unwrap(), records);
        store.integrity_check().unwrap();
    }
    temp.close().unwrap();
}

#[test]
fn logs_an_older_build_stored_are_decided_again_to_the_state_that_build_held() {
    for (name, bytes, pile, joined) in [
        ("host", STORED_HOST, [0, 9, 17, 19], 0),
        ("mirror", STORED_MIRROR, [6, 13, 21, 30], JOINS as i64),
    ] {
        for from_its_first_checkpoint in [false, true] {
            let temp = temporary();
            let path = temp.path().join(name);
            fs::create_dir(&path).unwrap();
            fs::write(path.join("project.sqlite"), bytes).unwrap();
            // As that build left it: a checkpoint at every request,
            // cancellation and catch-up, and no seal.
            assert_eq!(checkpoints(&path), pile, "{name}");
            assert_eq!(stored_seal(&path), None);
            // It decided every transaction as this build does.
            assert_eq!(outcome(&path), PINNED_LOG, "{name}");
            let older_notices = notices(&path);
            assert_eq!(older_notices.len(), 3);

            if from_its_first_checkpoint {
                query(&path)
                    .execute("DELETE FROM snapshots WHERE host_seq>?1", [joined])
                    .unwrap();
            }
            let mut store = ProjectStore::open(&path).unwrap();
            if from_its_first_checkpoint {
                // That build recorded the cancellation in its newest
                // checkpoint, which this variant took away. A cancellation
                // is the one thing the log cannot give back.
                assert!(!store.gesture_closed(&device(2), &id(CANCELLED)));
                store
                    .cancel_gesture(device(2), id(CANCELLED), 9_000)
                    .unwrap();
            }
            // The state is the one the script gives today, journal included.
            // For the other device as it was left, that state is read from
            // the older build's own checkpoint and nothing is decided again.
            assert_eq!(
                state(&store.checkpoint_bytes().unwrap()),
                PINNED_STATE,
                "{name} {from_its_first_checkpoint}"
            );
            let replayed = match (name, from_its_first_checkpoint) {
                ("mirror", false) => 0,
                _ => 30 - joined as u64,
            };
            assert_eq!(store.replayed_transactions(), replayed, "{name}");
            assert!(store.gesture_closed(&device(2), &id(CANCELLED)));
            assert_eq!(store.open_report().skipped_checkpoints, 0);
            store.integrity_check().unwrap();
            let held = store.checkpoint_bytes().unwrap();
            drop(store);
            // The notices are still the ones that build stored.
            assert_eq!(notices(&path), older_notices);
            let kept = match (name, from_its_first_checkpoint) {
                ("mirror", false) => 30,
                _ => joined,
            };
            assert_eq!(checkpoints(&path), [kept], "{name}");
            assert_eq!(stored_seal(&path).unwrap(), seal(&path));
            let store = ProjectStore::open(&path).unwrap();
            assert!(store.checkpoint_bytes().unwrap() == held);
            assert_eq!(store.replayed_transactions(), replayed, "{name}");
            drop(store);
            temp.close().unwrap();
        }
    }
}

#[test]
fn the_wording_and_number_of_notices_is_not_part_of_what_is_decided_again() {
    let temp = temporary();
    let (host_path, mirror_path, _) = run_script(&temp);
    // Another build worded the notice of revision 16 differently, gave none
    // at 28 and two at 30, and sealed what it stored.
    for path in [&host_path, &mirror_path] {
        query(path)
            .execute("DELETE FROM snapshots WHERE host_seq>?1", [JOINS as i64])
            .unwrap();
        rewrite_ack(path, 16, |ack| {
            ack.notices = vec!["Left one property as the other device set it".into()];
        });
        rewrite_ack(path, 28, |ack| ack.notices.clear());
        rewrite_ack(path, 30, |ack| {
            ack.notices = vec!["First".into(), "Second".into()];
        });
        reseal(path);
        let reworded = notices(path);
        assert_eq!(reworded.len(), 2);

        let mut store = ProjectStore::open(path).unwrap();
        assert_eq!(state(&store.checkpoint_bytes().unwrap()), PINNED_STATE);
        // The stored acknowledgements are kept as they are: in the project,
        // in what a retry is answered with, and in what the other device is
        // given.
        let hosting = store.local_device().unwrap() == *store.host_device();
        for seq in [16, 28, 30] {
            let (txn, ack) = store.accepted_transaction(&id(10_000 + seq)).unwrap();
            let expected = reworded
                .iter()
                .find(|(row, _)| *row == seq as i64)
                .map(|(_, notices)| notices.clone())
                .unwrap_or_default();
            assert_eq!(ack.notices, expected, "revision {seq}");
            if hosting {
                let (txn, author) = (txn.clone(), device(if seq == 28 { 2 } else { 1 }));
                let again = store.commit(&txn, &author, 9_000).unwrap();
                assert!(again.duplicate);
                assert_eq!(again.ack.notices, expected, "revision {seq}");
            }
        }
        let given = store.checkpoint_bytes().unwrap();
        drop(store);
        assert_eq!(notices(path), reworded);
        // A checkpoint of it reads back, and the project opens from it.
        let mut store = ProjectStore::open(path).unwrap();
        store.checkpoint_now(9_001).unwrap();
        drop(store);
        let store = ProjectStore::open(path).unwrap();
        assert_eq!(store.replayed_transactions(), 0);
        assert!(store.checkpoint_bytes().unwrap() == given);
        drop(store);
    }
    temp.close().unwrap();
}

#[test]
fn a_sealed_transaction_this_engine_decides_differently_is_another_version_and_nothing_changes() {
    type Change = fn(&Path);
    let another_hash: Change = |path| {
        // The build that saved this reached another state at revision 20.
        let hash = blake3::hash(b"another engine");
        rewrite_ack(path, 20, |ack| ack.state_hash = hash.as_bytes().to_vec());
        query(path)
            .execute(
                "UPDATE op_log SET state_hash=?1 WHERE host_seq=20",
                [hash.to_hex().as_str()],
            )
            .unwrap();
    };
    let refused_here: Change = |path| {
        // The build that saved this accepted what this engine refuses: here
        // a property this one does not know.
        let database = query(path);
        let bytes: Vec<u8> = database
            .query_row("SELECT txn FROM op_log WHERE host_seq=27", [], |row| {
                row.get(0)
            })
            .unwrap();
        let mut txn = pb::Transaction::decode(bytes.as_slice()).unwrap();
        let Some(pb::op::Kind::SetProperty(set)) = &mut txn.ops[0].kind else {
            panic!("revision 27 sets a property");
        };
        set.property = "style.shadow".into();
        database
            .execute(
                "UPDATE op_log SET txn=?1 WHERE host_seq=27",
                [txn.encode_to_vec()],
            )
            .unwrap();
    };
    let another_conflict: Change = |path| {
        // The build that saved this described the conflict of revision 24
        // differently, with the same resulting project.
        exchange_conflict_devices(path, "WHERE property LIKE 'layers.%'");
    };
    for (what, change, row) in [
        ("another state hash", another_hash, 20),
        ("a transaction refused here", refused_here, 27),
        ("another conflict record", another_conflict, 24),
    ] {
        let temp = temporary();
        let (host_path, mirror_path, _) = run_script(&temp);
        for path in [&host_path, &mirror_path] {
            // Both replay the row: it is after the host's checkpoint and
            // after the other device joined.
            change(path);
            reseal(path);
            let before = stored(path);
            assert!(
                matches!(ProjectStore::open(path), Err(StoreError::EngineMismatch)),
                "{what}"
            );
            assert_eq!(stored(path), before, "{what}");

            // The seal is what says the rows are as they were written. Where
            // it ends before the row, the same difference is damage, as it
            // was before there was a seal.
            set_seal(path, &seal_through(path, row - 1));
            let before = stored(path);
            assert!(
                matches!(ProjectStore::open(path), Err(StoreError::Corrupt(_))),
                "{what}"
            );
            assert_eq!(stored(path), before, "{what}");
        }
        temp.close().unwrap();
    }

    // A conflict record a checkpoint holds is not decided again, so one that
    // differs in the table is damage whatever the seal says.
    let temp = temporary();
    let (host_path, _, _) = run_script(&temp);
    exchange_conflict_devices(&host_path, "ORDER BY rowid LIMIT 1");
    assert!(matches!(
        ProjectStore::open(&host_path),
        Err(StoreError::Corrupt("conflict projection"))
    ));
    temp.close().unwrap();
}
