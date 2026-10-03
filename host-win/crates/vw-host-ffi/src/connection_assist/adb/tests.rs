use super::*;
use std::collections::VecDeque;

fn config() -> AdbReverseConfig {
    AdbReverseConfig {
        adb_path: "C:\\approved\\adb.exe".into(),
        serial: "selected-test-device".into(),
        phone_port: 47191,
        host_port: 47192,
    }
}
struct Fake {
    devices: Vec<AdbDevice>,
    lists: VecDeque<Vec<u8>>,
    created: usize,
    refuse: bool,
    targets: Vec<String>,
}
impl Fake {
    fn available(list: &[u8]) -> Self {
        Self {
            devices: vec![AdbDevice {
                serial: config().serial,
                state: AdbDeviceState::Available,
            }],
            lists: VecDeque::from([list.to_vec()]),
            created: 0,
            refuse: false,
            targets: vec![],
        }
    }
}
impl ReverseBackend for Fake {
    fn devices(&mut self) -> Result<Vec<AdbDevice>> {
        Ok(self.devices.clone())
    }
    fn list(&mut self, serial: &str) -> Result<Vec<u8>> {
        self.targets.push(serial.into());
        self.lists.pop_front().ok_or(Error::AdbProtocol)
    }
    fn create(&mut self, value: &AdbReverseConfig) -> Result<()> {
        self.targets.push(value.serial.clone());
        self.created += 1;
        if self.refuse {
            Err(Error::AdbRefused)
        } else {
            Ok(())
        }
    }
}
#[test]
fn creates_only_the_owner_selected_device_and_port_when_missing() -> Result<()> {
    let mut backend = Fake::available(b"host tcp:8000 tcp:8001\n");
    backend.devices.push(AdbDevice {
        serial: "other-project-phone".into(),
        state: AdbDeviceState::Available,
    });
    assert_eq!(
        cycle(&mut backend, &config())?,
        (AdbReverseState::MappingCreated, true)
    );
    assert_eq!(backend.created, 1);
    assert!(
        backend
            .targets
            .iter()
            .all(|serial| serial == &config().serial)
    );
    Ok(())
}
#[test]
fn an_existing_equal_mapping_is_observed_without_claiming_ownership() -> Result<()> {
    let mut backend = Fake::available(b"host tcp:47191 tcp:47192\n");
    assert_eq!(
        cycle(&mut backend, &config())?,
        (AdbReverseState::ExistingMapping, false)
    );
    assert_eq!(backend.created, 0);
    Ok(())
}
#[test]
fn another_destination_is_contested_and_never_overwritten() -> Result<()> {
    let mut backend = Fake::available(b"host tcp:47191 tcp:9000\n");
    assert_eq!(
        cycle(&mut backend, &config())?,
        (AdbReverseState::ContestedMapping, false)
    );
    assert_eq!(backend.created, 0);
    Ok(())
}
#[test]
fn no_rebind_race_reports_the_winner_without_retrying_an_overwrite() -> Result<()> {
    for (destination, state) in [
        (47192, AdbReverseState::ExistingMapping),
        (9000, AdbReverseState::ContestedMapping),
    ] {
        let mut backend = Fake::available(b"");
        backend.refuse = true;
        backend
            .lists
            .push_back(format!("host tcp:47191 tcp:{destination}\n").into_bytes());
        assert_eq!(cycle(&mut backend, &config())?, (state, false));
        assert_eq!(backend.created, 1);
    }
    Ok(())
}
#[test]
fn missing_offline_and_unauthorized_devices_never_receive_a_request() -> Result<()> {
    for (device, expected) in [
        (None, AdbReverseState::MissingDevice),
        (Some(AdbDeviceState::Offline), AdbReverseState::Offline),
        (
            Some(AdbDeviceState::Unauthorized),
            AdbReverseState::Unauthorized,
        ),
    ] {
        let mut backend = Fake::available(b"");
        backend.devices.clear();
        if let Some(state) = device {
            backend.devices.push(AdbDevice {
                serial: config().serial,
                state,
            });
        }
        assert_eq!(cycle(&mut backend, &config())?, (expected, false));
        assert!(backend.targets.is_empty());
    }
    Ok(())
}
#[test]
fn selected_device_reappearance_and_dropped_mapping_are_recreated() -> Result<()> {
    let mut backend = Fake::available(b"");
    assert_eq!(
        cycle(&mut backend, &config())?.0,
        AdbReverseState::MappingCreated
    );
    backend.devices.clear();
    assert_eq!(
        cycle(&mut backend, &config())?.0,
        AdbReverseState::MissingDevice
    );
    backend.devices.push(AdbDevice {
        serial: config().serial,
        state: AdbDeviceState::Available,
    });
    backend.lists.push_back(vec![]);
    assert_eq!(
        cycle(&mut backend, &config())?.0,
        AdbReverseState::MappingCreated
    );
    assert_eq!(backend.created, 2);
    // This is a scheduling arithmetic test, not a hardware timing measurement.
    assert!(POLL + CYCLE < Duration::from_secs(1));
    Ok(())
}
#[test]
fn inventory_parser_is_bounded_exact_and_duplicate_safe() -> Result<()> {
    let devices = parse_devices(b"test-one\tdevice\ntest-two\toffline\n")?;
    assert_eq!(devices.len(), 2);
    assert_eq!(devices[0].state, AdbDeviceState::Available);
    assert!(parse_devices(b"same\tdevice\nsame\toffline\n").is_err());
    assert!(parse_devices(b"bad serial\tdevice\n").is_err());
    assert!(parse_devices(b"bad;host:kill\tdevice\n").is_err());
    assert!(parse_devices(&vec![b'a'; MAX_REPLY + 1]).is_err());
    assert!(
        parse_devices(
            (0..65)
                .map(|n| format!("test-{n}\tdevice\n"))
                .collect::<String>()
                .as_bytes()
        )
        .is_err()
    );
    Ok(())
}
#[test]
fn malformed_mapping_cannot_hide_an_occupied_port() {
    for bytes in [
        b"host tcp:47191 tcp:9000\nhost tcp:47191 tcp:47192\n".as_slice(),
        b"unknown tcp:47191 tcp:9000\n",
        b"host tcp:47191\n",
    ] {
        assert!(mapping(bytes, 47191, 47192).is_err());
    }
}
#[test]
fn framing_matches_the_official_smart_socket_encoding() -> Result<()> {
    assert_eq!(encode_request("host:version")?, b"000chost:version");
    assert_eq!(decode_length(*b"0029")?, 41);
    assert_eq!(decode_length(*b"ffff")?, 65_535);
    assert!(decode_length(*b"-001").is_err());
    assert!(decode_length(*b"GGGG").is_err());
    assert!(encode_request("host:devices\n").is_err());
    Ok(())
}
#[test]
fn serial_selection_cannot_inject_a_second_adb_service() {
    for serial in ["", "a\0b", "a;b", "a\nb", "a b", "a\tb"] {
        assert!(validate_serial(serial).is_err());
    }
    assert!(validate_serial("emulator-5554").is_ok());
    assert!(validate_serial("192.0.2.1:5555").is_ok());
    assert!(validate_serial(&"x".repeat(129)).is_err());
}
#[test]
fn cancellation_token_is_one_shot_and_never_starts_after_cancel() {
    let request = ConnectionRequest::new();
    request.cancel();
    assert_eq!(request.begin(), Err(Error::Cancelled));
    let request = ConnectionRequest::new();
    assert!(request.begin().is_ok());
    assert_eq!(request.begin(), Err(Error::AlreadyUsed));
}
#[test]
fn watch_stop_keeps_created_mapping_ownership_history() -> Result<()> {
    let stop = ConnectionRequest::new();
    let snapshot = Arc::new(Mutex::new(AdbReverseSnapshot {
        state: AdbReverseState::MappingCreated,
        created_count: 1,
        cycle_millis: 12,
        created_mapping_may_remain: true,
        phone_port: 47191,
        host_port: 47192,
    }));
    let (_done, receiver) = watch::channel(false);
    let watch = AdbReverseWatch {
        stop: stop.clone(),
        snapshot: snapshot.clone(),
        done: receiver,
    };
    drop(watch);
    assert_eq!(stop.check(), Err(Error::Cancelled));
    assert!(
        snapshot
            .lock()
            .map_err(|_| Error::WorkerUnavailable)?
            .created_mapping_may_remain
    );
    Ok(())
}
