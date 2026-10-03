//! Deterministic cancellation at the actual asynchronous authorization boundary.
#![allow(clippy::unwrap_used, clippy::panic)]
use super::*;
use std::{
    future::poll_fn,
    sync::{atomic::AtomicUsize, mpsc},
    task::Poll,
};
use tokio::sync::oneshot;

struct GatedPolicy {
    calls: AtomicUsize,
    block_on: usize,
    entered: StdMutex<Option<oneshot::Sender<()>>>,
    release: StdMutex<mpsc::Receiver<()>>,
    revoked: AtomicBool,
}
impl PeerAuthorization for GatedPolicy {
    fn authorize(&self, _: &DeviceId, _: &[u8]) -> Result<()> {
        let call = self.calls.fetch_add(1, Ordering::AcqRel) + 1;
        if call == self.block_on {
            if let Some(entered) = self.entered.lock().unwrap().take() {
                let _ = entered.send(());
            }
            self.release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(3))
                .map_err(|_| NetError::Timeout)?;
        }
        if self.revoked.load(Ordering::Acquire) {
            Err(NetError::Authentication)
        } else {
            Ok(())
        }
    }
}
// An assertion failure also releases the blocking worker before runtime teardown.
struct Release(Option<mpsc::Sender<()>>);
impl Release {
    fn now(&mut self) {
        if let Some(release) = self.0.take() {
            let _ = release.send(());
        }
    }
}
impl Drop for Release {
    fn drop(&mut self) {
        self.now();
    }
}
fn fixture(
    block_on: usize,
) -> (
    CarrierReceiver,
    Arc<GatedPolicy>,
    oneshot::Receiver<()>,
    Release,
) {
    let (entered, ready) = oneshot::channel();
    let (release, released) = mpsc::channel();
    let policy = Arc::new(GatedPolicy {
        calls: AtomicUsize::new(0),
        block_on,
        entered: StdMutex::new(Some(entered)),
        release: StdMutex::new(released),
        revoked: AtomicBool::new(false),
    });
    let trusted = TrustedPeer {
        device: DeviceId::from_bytes([9; 16]),
        certificate: CertificateDer::from(vec![1, 2, 3]),
        server_name: "synthetic.invalid".into(),
    };
    let peer = AuthenticatedPeer {
        device: trusted.device.clone(),
        binding: [0; 32],
        authorization: Some(Arc::new(crate::carrier_authorization::Authorization {
            peer: trusted,
            policy: Some(policy.clone()),
        })),
    };
    let ingress = crate::ingress::Ingress::new();
    let life = Lifetime::new(ingress.clone(), None, None, None);
    (
        CarrierReceiver {
            ingress,
            life,
            peer,
            pending: None,
            authorization: None,
            read_authorized: false,
        },
        policy,
        ready,
        Release(Some(release)),
    )
}
fn reliable(sequence: u64) -> Frame {
    Frame {
        channel: v1::Channel::Ops,
        envelope: v1::Envelope {
            channel: v1::Channel::Ops as i32,
            seq: sequence,
            connection_id: Some(
                vw_model::Id::from_parts(1_700_000_000_000, [3; 10])
                    .unwrap()
                    .to_proto(),
            ),
            body: Some(Body::SyncRequest(v1::SyncRequest {
                since_host_seq: sequence,
                ..Default::default()
            })),
        },
    }
}
async fn cancel_at_gate(receiver: &mut CarrierReceiver, ready: oneshot::Receiver<()>) {
    let mut receiving = Box::pin(receiver.receive());
    tokio::select! {
        result = &mut receiving => panic!("receive escaped the policy gate: {result:?}"),
        entered = tokio::time::timeout(Duration::from_secs(3), ready) => {
            entered.unwrap().unwrap();
        }
    }
    drop(receiving);
}
async fn poll_then_cancel(receiver: &mut CarrierReceiver) {
    let mut receiving = Box::pin(receiver.receive());
    assert!(poll_fn(|cx| Poll::Ready(receiving.as_mut().poll(cx).is_pending())).await);
    drop(receiving);
}
async fn receive_bounded(receiver: &mut CarrierReceiver) -> Result<Frame> {
    tokio::time::timeout(Duration::from_secs(3), receiver.receive())
        .await
        .unwrap()
}

#[tokio::test(flavor = "current_thread")]
async fn cancelled_post_dequeue_authorization_preserves_exact_reliable_order() {
    let (mut receiver, policy, ready, mut release) = fixture(2);
    let first = reliable(1);
    let second = reliable(2);
    receiver.ingress.push(first.clone()).unwrap();
    receiver.ingress.push(second.clone()).unwrap();
    cancel_at_gate(&mut receiver, ready).await;
    assert!(receiver.pending.is_some());
    for _ in 0..8 {
        poll_then_cancel(&mut receiver).await;
    }
    assert_eq!(policy.calls.load(Ordering::Acquire), 2);
    release.now();
    assert_eq!(
        receive_bounded(&mut receiver).await.unwrap().envelope,
        first.envelope
    );
    assert_eq!(policy.calls.load(Ordering::Acquire), 2);
    assert_eq!(
        receive_bounded(&mut receiver).await.unwrap().envelope,
        second.envelope
    );
    assert_eq!(policy.calls.load(Ordering::Acquire), 4);
    receiver.close();
}

#[tokio::test(flavor = "current_thread")]
async fn cancelled_pre_dequeue_authorization_is_resumed_without_duplicate_workers() {
    let (mut receiver, policy, ready, mut release) = fixture(1);
    let expected = reliable(1);
    receiver.ingress.push(expected.clone()).unwrap();
    cancel_at_gate(&mut receiver, ready).await;
    assert!(receiver.pending.is_none());
    for _ in 0..8 {
        poll_then_cancel(&mut receiver).await;
    }
    assert_eq!(policy.calls.load(Ordering::Acquire), 1);
    release.now();
    assert_eq!(
        receive_bounded(&mut receiver).await.unwrap().envelope,
        expected.envelope
    );
    assert_eq!(policy.calls.load(Ordering::Acquire), 2);
    receiver.close();
}

#[tokio::test(flavor = "current_thread")]
async fn revoked_peer_cannot_receive_a_frame_retained_across_cancellation() {
    let (mut receiver, policy, ready, mut release) = fixture(2);
    receiver.ingress.push(reliable(1)).unwrap();
    cancel_at_gate(&mut receiver, ready).await;
    assert!(receiver.pending.is_some());
    policy.revoked.store(true, Ordering::Release);
    release.now();
    assert!(matches!(
        receive_bounded(&mut receiver).await,
        Err(NetError::Authentication)
    ));
    assert!(receiver.pending.is_none());
    assert!(receiver.ingress.is_failed());
    assert!(matches!(
        receive_bounded(&mut receiver).await,
        Err(NetError::Carrier)
    ));
    assert_eq!(policy.calls.load(Ordering::Acquire), 2);
}
