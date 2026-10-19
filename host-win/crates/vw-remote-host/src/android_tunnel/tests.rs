use super::{Error, capture, device, mirror, session};
use std::{collections::VecDeque, sync::{Arc, Mutex}, time::{Duration, SystemTime, UNIX_EPOCH}};

const DEVICES: &str = "List of devices attached\nUSB123\tdevice usb:1-1 model:SM_S918U transport_id:7\n\n";

fn selection() -> super::Result<device::UsbSelection> {
    device::UsbSelection::new("SM_S918U".into(), 7)
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

fn reply(code: u32, bytes: &[u8]) -> session::CommandOutput {
    session::CommandOutput { code, bytes: bytes.to_vec(), started_ms: now(), finished_ms: now() }
}

fn chunk(out: &mut Vec<u8>, tag: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(tag);
    out.extend_from_slice(data);
    let mut crc = !0u32;
    for byte in tag.iter().chain(data) {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320u32 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    out.extend_from_slice(&(!crc).to_be_bytes());
}

fn png() -> Vec<u8> {
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut out, b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0]);
    chunk(&mut out, b"IDAT", &[0x78, 1, 1, 5, 0, 0xfa, 0xff, 0, 0, 0, 0, 0, 0, 5, 0, 1]);
    chunk(&mut out, b"IEND", &[]);
    out
}

#[test]
fn explicit_usb_selection_refuses_injection_ambiguity_and_status_substitution() -> super::Result<()> {
    assert_eq!(session::AndroidTunnelOwner::open(None,
        session::OwnerAdb::select("/tmp/platform-tools/adb.exe".into())?,
        selection()?, [1; 16], 1, &session::Cancellation::default()).err(),
        Some(Error::RuntimeUnavailable));
    assert!(session::OwnerAdb::select("adb.exe".into()).is_err());
    assert!(session::OwnerAdb::select("/tmp/scrcpy/adb.exe".into()).is_err());
    assert!(device::UsbSelection::new("SM S918U".into(), 7).is_err());
    assert!(device::UsbSelection::new("SM_S918U".into(), 0).is_err());
    let selected = device::select(DEVICES.as_bytes(), &selection()?)?;
    device::still_selected(&selected, DEVICES.as_bytes())?;
    assert_eq!(device::select(b"List of devices attached\nUSB123\tunauthorized usb:1 model:SM_S918U transport_id:7\n", &selection()?).err(), Some(Error::UnauthorizedDevice));
    assert_eq!(device::select(b"List of devices attached\nUSB123\tdevice usb:1 model:SM_S918U transport_id:7\nUSB124\tdevice usb:2 model:SM_S918U transport_id:7\n", &selection()?).err(), Some(Error::AmbiguousDevice));
    assert!(device::select(b"List of devices attached\n192.0.2.1:5555\tdevice model:SM_S918U transport_id:7\n", &selection()?).is_err());
    assert!(device::still_selected(&selected, b"List of devices attached\nUSB124\tdevice usb:1 model:SM_S918U transport_id:7\n").is_err());
    Ok(())
}

#[test]
fn png_and_xml_refuse_truncation_budgets_entities_and_invalid_bounds() -> super::Result<()> {
    let original = png();
    let parsed = capture::png(original.clone())?;
    assert_eq!((parsed.width, parsed.height, parsed.decoded_budget), (1, 1, 4));
    let mut truncated = original.clone();
    truncated.pop();
    assert!(capture::png(truncated).is_err());
    let mut bad_crc = original;
    bad_crc[20] ^= 1;
    assert!(capture::png(bad_crc).is_err());
    assert_eq!(capture::png(vec![0; capture::MAX_PNG + 1]).err(), Some(Error::Limit));
    let xml = br#"<hierarchy><node bounds="[0,0][1,1]" class="android.widget.TextView" text="hello"/></hierarchy>"#;
    assert_eq!(capture::hierarchy(xml, 1, 1)?[0].text, "hello");
    assert!(capture::hierarchy(br#"<!DOCTYPE hierarchy [<!ENTITY x SYSTEM "file:///tmp/x">]><hierarchy/>"#, 1, 1).is_err());
    assert!(capture::hierarchy(br#"<hierarchy><node bounds="[0,0][2,1]"/></hierarchy>"#, 1, 1).is_err());
    let deep = format!("<hierarchy>{}</hierarchy>", "<node bounds=\"[0,0][1,1]\">".repeat(35)
        + &"</node>".repeat(35));
    assert_eq!(capture::hierarchy(deep.as_bytes(), 1, 1).err(), Some(Error::Limit));
    Ok(())
}

#[test]
fn mirror_census_uses_signed_physical_intersection_and_refuses_unknown() -> super::Result<()> {
    let source = mirror::PhysicalRect { left: -1900, top: -800, right: -100, bottom: 200 };
    let mirror_window = mirror::MirrorWindow { hwnd: 1,
        rect: mirror::PhysicalRect { left: -300, top: -200, right: 100, bottom: 300 },
        visible: true, child_of_source: false, source_child_of_mirror: false };
    let mut snapshot = mirror::Snapshot { selected: source, mirrors: vec![mirror_window], complete: true };
    assert_eq!(mirror::check(&snapshot), Err(Error::FeedbackLoop));
    snapshot.mirrors[0].rect.left = 200;
    snapshot.mirrors[0].rect.right = 300;
    mirror::check(&snapshot)?;
    snapshot.mirrors[0].child_of_source = true;
    assert_eq!(mirror::check(&snapshot), Err(Error::FeedbackLoop));
    snapshot.complete = false;
    assert_eq!(mirror::check(&snapshot), Err(Error::Platform));
    Ok(())
}

struct MockState {
    queue: VecDeque<(Vec<String>, session::CommandOutput)>,
    seen: Vec<Vec<String>>,
    close_failures: usize,
    closes: usize,
    active_io: bool,
    stall_on: Option<Vec<String>>,
    settle_failures: usize,
    settles: usize,
}

struct MockRunner(Arc<Mutex<MockState>>);

impl session::Runner for MockRunner {
    fn run(&mut self, args: &[String], cap: usize, _timeout: Duration, cancel: &session::Cancellation)
        -> super::Result<session::CommandOutput> {
        cancel.check()?;
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.active_io { return Err(Error::RetirementPending); }
        let (expected, output) = state.queue.pop_front().ok_or(Error::Platform)?;
        assert_eq!(args, expected);
        assert!(output.bytes.len() <= cap);
        state.seen.push(args.to_vec());
        if state.stall_on.as_ref().is_some_and(|v| v.as_slice() == args) {
            state.stall_on = None;
            state.active_io = true;
            return Err(Error::Timeout);
        }
        Ok(output)
    }
    fn launch_mirror(&mut self, serial: &str, _: &session::Cancellation) -> super::Result<u32> {
        assert_eq!(serial, "USB123");
        Ok(4242)
    }
    fn close(&mut self) -> super::Result<()> {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        assert!(!state.active_io);
        state.closes += 1;
        if state.close_failures > 0 { state.close_failures -= 1; Err(Error::RetirementPending) }
        else { Ok(()) }
    }
    fn settle_io(&mut self) -> super::Result<()> {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        state.settles += 1;
        if state.settle_failures > 0 {
            state.settle_failures -= 1;
            return Err(Error::RetirementPending);
        }
        state.active_io = false;
        Ok(())
    }
}

struct MockProbe;
impl mirror::WindowProbe for MockProbe {
    fn current(&mut self, _: mirror::CaptureSource, pid: u32) -> super::Result<mirror::Snapshot> {
        assert_eq!(pid, 4242);
        Ok(mirror::Snapshot { selected: mirror::PhysicalRect { left: 0, top: 0, right: 1, bottom: 1 },
            mirrors: vec![mirror::MirrorWindow { hwnd: 8,
                rect: mirror::PhysicalRect { left: 2, top: 2, right: 3, bottom: 3 },
                visible: true, child_of_source: false, source_child_of_mirror: false }], complete: true })
    }
}

#[test]
fn owner_pairs_exact_commands_and_retains_unknown_close_until_retry() -> super::Result<()> {
    assert_eq!(session::checked_completed_age(1_000, 1_010, 11_000, 11_000)?, 10_000);
    assert_eq!(session::checked_completed_age(1_000, 1_010, 900, 11_000).err(), Some(Error::Stale));
    assert_eq!(session::checked_completed_age(1_000, 1_010, 11_000, 10_000).err(), Some(Error::Stale));
    let session = [9u8; 16];
    let nonce = "09".repeat(16);
    let scratch = format!("/sdcard/vw-{nonce}-3");
    let xml_path = format!("{scratch}/hierarchy.xml");
    let id = |tail: &[&str]| {
        let mut argv = vec!["-t".to_owned(), "7".to_owned()];
        argv.extend(tail.iter().map(|word| (*word).to_owned()));
        argv
    };
    let listed = || (vec!["devices".into(), "-l".into()], reply(0, DEVICES.as_bytes()));
    let xml = br#"<hierarchy><node bounds="[0,0][1,1]" text="owned"/></hierarchy>"#;
    let state = Arc::new(Mutex::new(MockState { queue: VecDeque::from([
        listed(), listed(), (id(&["exec-out", "screencap", "-p"]), reply(0, &png())),
        listed(), (id(&["shell", "mkdir", &scratch]), reply(0, b"")),
        (id(&["shell", "uiautomator", "dump", &xml_path]), reply(0, b"")),
        listed(), (id(&["exec-out", "cat", &xml_path]), reply(0, xml)),
        listed(), (id(&["shell", "rm", "-f", &xml_path]), reply(0, b"")),
        (id(&["shell", "test", "-e", &scratch]), reply(0, b"")),
        (id(&["shell", "rmdir", &scratch]), reply(0, b"")),
        (id(&["shell", "test", "-e", &scratch]), reply(1, b"")), listed(),
    ]), seen: vec![], close_failures: 1, closes: 0, active_io: false,
        stall_on: None, settle_failures: 0, settles: 0 }));
    let cancel = session::Cancellation::default();
    let mut owner = session::AndroidTunnelOwner::open_with(Box::new(MockRunner(state.clone())),
        Box::new(MockProbe), selection()?, session, 3, &cancel)?;
    owner.check_source(mirror::CaptureSource::Window { hwnd: 1 })?;
    let pair = owner.capture_pair(&cancel)?;
    assert_eq!(pair.session, session);
    assert_eq!(pair.source_generation, 3);
    assert!(matches!(pair.hierarchy, capture::Hierarchy::Present { .. }));
    cancel.cancel();
    assert_eq!(owner.capture_pair(&cancel).err(), Some(Error::Cancelled));
    assert_eq!(owner.close(), Err(Error::RetirementPending));
    assert_eq!(owner.close(), Ok(()));
    let state = state.lock().unwrap_or_else(|e| e.into_inner());
    assert!(state.queue.is_empty());
    assert_eq!(state.closes, 2);
    assert!(state.seen.iter().all(|argv| !argv.iter().any(|word| word.contains("adb.exe"))));
    drop(state);

    let second_session = [8u8; 16];
    let scratch = format!("/sdcard/vw-{}-4", "08".repeat(16));
    let xml_path = format!("{scratch}/hierarchy.xml");
    let second = Arc::new(Mutex::new(MockState { queue: VecDeque::from([
        listed(), listed(), (id(&["exec-out", "screencap", "-p"]), reply(0, &png())),
        listed(), (id(&["shell", "mkdir", &scratch]), reply(0, b"")),
        (id(&["shell", "uiautomator", "dump", &xml_path]), reply(0, b"")),
        listed(), (id(&["exec-out", "cat", &xml_path]), reply(0, xml)),
        listed(), (id(&["shell", "rm", "-f", &xml_path]), reply(0, b"")),
        (id(&["shell", "test", "-e", &scratch]), reply(0, b"")),
        (id(&["shell", "rmdir", &scratch]), reply(0, b"")),
        (id(&["shell", "test", "-e", &scratch]), reply(2, b"")),
        listed(),
        (id(&["shell", "test", "-e", &scratch]), reply(1, b"")),
    ]), seen: vec![], close_failures: 0, closes: 0, active_io: false,
        stall_on: None, settle_failures: 0, settles: 0 }));
    let fresh = session::Cancellation::default();
    let mut pending = session::AndroidTunnelOwner::open_with(Box::new(MockRunner(second.clone())),
        Box::new(MockProbe), selection()?, second_session, 4, &fresh)?;
    assert_eq!(pending.capture_pair(&fresh).err(), Some(Error::RetirementPending));
    assert_eq!(pending.capture_pair(&fresh).err(), Some(Error::RetirementPending));
    let contender = Arc::new(Mutex::new(MockState { queue: VecDeque::new(), seen: vec![],
        close_failures: 0, closes: 0, active_io: false, stall_on: None,
        settle_failures: 0, settles: 0 }));
    assert_eq!(session::AndroidTunnelOwner::open_with(Box::new(MockRunner(contender)),
        Box::new(MockProbe), selection()?, [7; 16], 5, &fresh).err(),
        Some(Error::RetirementPending));
    pending.close()?;
    let second = second.lock().unwrap_or_else(|e| e.into_inner());
    assert!(second.queue.is_empty());
    assert_eq!(second.seen.iter().filter(|argv| argv.iter().any(|v| v == "rmdir")).count(), 1);
    drop(second);

    let third_session = [6u8; 16];
    let directory = format!("/sdcard/vw-{}-6", "06".repeat(16));
    let third = Arc::new(Mutex::new(MockState { queue: VecDeque::from([
        listed(), listed(), (id(&["exec-out", "screencap", "-p"]), reply(0, &png())),
        listed(), (id(&["shell", "mkdir", &directory]), reply(1, b"")),
        listed(), (id(&["shell", "test", "-e", &directory]), reply(1, b"")),
    ]), seen: vec![], close_failures: 0, closes: 0, active_io: false,
        stall_on: None, settle_failures: 0, settles: 0 }));
    let mut absent = session::AndroidTunnelOwner::open_with(Box::new(MockRunner(third.clone())),
        Box::new(MockProbe), selection()?, third_session, 6, &fresh)?;
    assert_eq!(absent.capture_pair(&fresh).err(), Some(Error::Storage));
    absent.close()?;
    assert!(third.lock().unwrap_or_else(|e| e.into_inner()).queue.is_empty());

    let fourth_session = [5u8; 16];
    let directory = format!("/sdcard/vw-{}-7", "05".repeat(16));
    let xml_path = format!("{directory}/hierarchy.xml");
    let stalled = id(&["shell", "uiautomator", "dump", &xml_path]);
    let fourth = Arc::new(Mutex::new(MockState { queue: VecDeque::from([
        listed(), listed(), (id(&["exec-out", "screencap", "-p"]), reply(0, &png())),
        listed(), (id(&["shell", "mkdir", &directory]), reply(0, b"")),
        (stalled.clone(), reply(0, b"")),
        listed(), (id(&["shell", "rm", "-f", &xml_path]), reply(0, b"")),
        (id(&["shell", "test", "-e", &directory]), reply(0, b"")),
        (id(&["shell", "rmdir", &directory]), reply(0, b"")),
        (id(&["shell", "test", "-e", &directory]), reply(1, b"")),
    ]), seen: vec![], close_failures: 0, closes: 0, active_io: false,
        stall_on: Some(stalled), settle_failures: 2, settles: 0 }));
    let mut active = session::AndroidTunnelOwner::open_with(Box::new(MockRunner(fourth.clone())),
        Box::new(MockProbe), selection()?, fourth_session, 7, &fresh)?;
    assert_eq!(active.capture_pair(&fresh).err(), Some(Error::RetirementPending));
    assert_eq!(active.close(), Err(Error::RetirementPending));
    assert_eq!(fourth.lock().unwrap_or_else(|e| e.into_inner()).closes, 0);
    active.close()?;
    let fourth = fourth.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(fourth.settles, 3);
    assert_eq!(fourth.closes, 1);
    assert!(fourth.queue.is_empty());
    drop(fourth);

    let fifth_session = [4u8; 16];
    let directory = format!("/sdcard/vw-{}-8", "04".repeat(16));
    let xml_path = format!("{directory}/hierarchy.xml");
    let base = now();
    let mut image = reply(0, &png());
    image.started_ms = base - 10_000;
    image.finished_ms = base - 10_000;
    let mut dump = reply(0, b"");
    dump.started_ms = base - 9_990;
    dump.finished_ms = base - 5_000;
    let mut claim = reply(0, b"");
    claim.started_ms = base - 9_998;
    claim.finished_ms = base - 9_995;
    let mut hierarchy = reply(0, xml);
    hierarchy.started_ms = base - 5_000;
    hierarchy.finished_ms = base - 10;
    let fifth = Arc::new(Mutex::new(MockState { queue: VecDeque::from([
        listed(), listed(), (id(&["exec-out", "screencap", "-p"]), image),
        listed(), (id(&["shell", "mkdir", &directory]), claim),
        (id(&["shell", "uiautomator", "dump", &xml_path]), dump),
        listed(), (id(&["exec-out", "cat", &xml_path]), hierarchy),
        listed(), (id(&["shell", "rm", "-f", &xml_path]), reply(0, b"")),
        (id(&["shell", "test", "-e", &directory]), reply(0, b"")),
        (id(&["shell", "rmdir", &directory]), reply(0, b"")),
        (id(&["shell", "test", "-e", &directory]), reply(1, b"")), listed(),
    ]), seen: vec![], close_failures: 0, closes: 0, active_io: false,
        stall_on: None, settle_failures: 0, settles: 0 }));
    let mut stale = session::AndroidTunnelOwner::open_with(Box::new(MockRunner(fifth.clone())),
        Box::new(MockProbe), selection()?, fifth_session, 8, &fresh)?;
    let pair = stale.capture_pair(&fresh)?;
    assert!(matches!(pair.hierarchy, capture::Hierarchy::Stale { age_ms } if age_ms >= 10_000));
    stale.close()?;
    assert!(fifth.lock().unwrap_or_else(|e| e.into_inner()).queue.is_empty());

    #[cfg(not(windows))]
    {
        let sixth = Arc::new(Mutex::new(MockState { queue: VecDeque::from([listed()]),
            seen: vec![], close_failures: 0, closes: 0, active_io: false,
            stall_on: None, settle_failures: 0, settles: 0 }));
        let mut owner = session::AndroidTunnelOwner::open_with(Box::new(MockRunner(sixth.clone())),
            Box::new(MockProbe), selection()?, [3; 16], 9, &fresh)?;
        session::synthetic_startups_for_test(1);
        assert_eq!(owner.close(), Err(Error::RetirementPending));
        assert_eq!(owner.close(), Err(Error::RetirementPending));
        assert_eq!(sixth.lock().unwrap_or_else(|e| e.into_inner()).closes, 1);
        assert_eq!(session::retry_retirements()?, 1);
        let blocked = Arc::new(Mutex::new(MockState { queue: VecDeque::new(), seen: vec![],
            close_failures: 0, closes: 0, active_io: false, stall_on: None,
            settle_failures: 0, settles: 0 }));
        assert_eq!(session::AndroidTunnelOwner::open_with(Box::new(MockRunner(blocked)),
            Box::new(MockProbe), selection()?, [3; 16], 9, &fresh).err(),
            Some(Error::RetirementPending));
        session::synthetic_startups_for_test(0);
        owner.close()?;
        assert_eq!(session::retry_retirements()?, 0);
    }
    Ok(())
}
