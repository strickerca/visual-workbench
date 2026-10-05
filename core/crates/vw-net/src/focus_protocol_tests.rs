#![allow(clippy::unwrap_used)]
use super::*;
use std::collections::BTreeSet;
use vw_model::{DeviceId, Id};
use vw_proto::v1 as pb;

#[test]
fn current_minor_explicitly_refuses_legacy_hellos_and_acks() {
    assert_eq!(PROTOCOL_MINOR, 6);
    let a = LocalHello {
        device: DeviceId::from_bytes([1; 16]),
        platform: pb::Platform::Windows,
        app_version: "fixture".into(),
        label: String::new(),
        capabilities: BTreeSet::from(["marker_focus_v1".into()]),
    };
    let b = LocalHello {
        device: DeviceId::from_bytes([2; 16]),
        platform: pb::Platform::Android,
        app_version: "fixture".into(),
        label: String::new(),
        capabilities: a.capabilities.clone(),
    };
    let id = Id::from_parts(1_700_000_000_000, [7; 10]).unwrap();
    let peer_a = AuthenticatedPeer::for_simulation(a.device.clone());
    let peer_b = AuthenticatedPeer::for_simulation(b.device.clone());
    for minor in 0..PROTOCOL_MINOR {
        let mut hello = a.hello(&id);
        hello.protocol_minor = minor;
        assert!(Session::accept(&b, &peer_a, &hello).is_err());
    }
    let (_, ack) = Session::accept(&b, &peer_a, &a.hello(&id)).unwrap();
    assert!(Session::finish(&a, &peer_b, id.clone(), &ack).is_ok());
    for minor in 0..PROTOCOL_MINOR {
        let mut legacy = ack.clone();
        legacy.protocol_minor = minor;
        assert!(Session::finish(&a, &peer_b, id.clone(), &legacy).is_err());
    }
}
