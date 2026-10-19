use vw_capture::{
    Error, Rect,
    picker::{MonitorTarget, PickerSource},
    stream::{
        CaptureStamp, OfferFeedback, ProducerReply, StreamProducer, StreamRequest, StreamSource,
    },
};
use vw_stream::{Cancellation, SessionBudget};

fn request() -> StreamRequest {
    StreamRequest {
        explicit_owner_action: true,
        owner_process_id: 42,
        capture_session_id: vw_model::Id::from_parts(1_700_000_000_000, [7; 10])
            .expect("fixture id")
            .to_string(),
        connection_epoch: 2,
        source_generation: 3,
        source: PickerSource::Monitor {
            target: MonitorTarget {
                monitor: 1,
                device: "synthetic-display".into(),
                physical: Rect {
                    x: -512,
                    y: -256,
                    width: 512,
                    height: 512,
                },
                dpi: 192,
                rotation: 0,
                primary: true,
                observed_ns: 100,
            },
        },
        geometry_revision: 4,
        target_fps: 15,
        endpoint_bytes: 128 * 1024 * 1024,
        ring_bytes: 16 * 1024 * 1024,
    }
}
struct Synthetic {
    next: u64,
    prepared: bool,
    reads: usize,
    discards: usize,
}
impl Synthetic {
    fn new() -> Self {
        Self {
            next: 100,
            prepared: false,
            reads: 0,
            discards: 0,
        }
    }
}
impl StreamSource for Synthetic {
    fn prepare(&mut self, _: u64, _: &Cancellation) -> vw_capture::Result<Option<CaptureStamp>> {
        self.next += 100;
        self.prepared = true;
        Ok(Some(CaptureStamp {
            timestamp_ns: self.next,
            captured_at_ms: 1_700_000_000_000,
        }))
    }
    fn read_into(&mut self, destination: &mut [u8], _: &Cancellation) -> vw_capture::Result<()> {
        assert!(self.prepared);
        self.reads += 1;
        for p in destination.as_chunks_mut::<4>().0 {
            p.copy_from_slice(&[42, 84, 126, 255]);
        }
        Ok(())
    }
    fn discard(&mut self) {
        self.prepared = false;
        self.discards += 1;
    }
}

#[test]
fn explicit_source_generation_and_ring_budget_refuse_before_capture() {
    let mut invalid = request();
    invalid.source_generation = 0;
    assert!(matches!(invalid.validate(), Err(Error::Invalid)));
    invalid = request();
    invalid.ring_bytes = 1024;
    assert!(matches!(invalid.validate(), Err(Error::Limit)));
    invalid = request();
    invalid.explicit_owner_action = false;
    assert!(matches!(invalid.validate(), Err(Error::Consent)));
    invalid = request();
    invalid.target_fps = 14;
    assert!(matches!(invalid.validate(), Err(Error::Invalid)));
}

#[test]
fn published_feedback_repairs_a_dropped_keyframe_without_relabeling_frames() {
    let request = request();
    let budget = SessionBudget::new(request.endpoint_bytes).expect("budget");
    let mut actor = StreamProducer::open(request, budget.clone()).expect("producer");
    let mut source = Synthetic::new();
    let first = actor
        .poll(&mut source, 1, None, false, 100)
        .expect("first poll");
    assert!(matches!(
        first,
        ProducerReply::Tiles {
            frame_id: 1,
            keyframe: true,
            ..
        }
    ));
    actor.published(1).expect("actual pipe publication");
    drop(first);
    let second = actor
        .poll(
            &mut source,
            2,
            Some(OfferFeedback {
                offer_sequence: 1,
                admitted_to_media: false,
            }),
            true,
            200,
        )
        .expect("repair poll");
    assert!(matches!(
        second,
        ProducerReply::Tiles {
            frame_id: 2,
            keyframe: true,
            ..
        }
    ));
    assert_eq!(source.reads, 2);
    assert_eq!(source.discards, 2);
    assert!(budget.used_bytes() > 0);
    assert!(matches!(
        actor.poll(&mut source, 3, None, false, 300),
        Err(Error::Invalid)
    ));
}

#[test]
fn original_requires_exact_frame_geometry_and_timestamp_after_eviction() {
    let request = request();
    let budget = SessionBudget::new(request.endpoint_bytes).expect("budget");
    let mut actor = StreamProducer::open(request.clone(), budget).expect("producer");
    let mut source = Synthetic::new();
    for sequence in 1..=9 {
        let feedback = (sequence > 1).then_some(OfferFeedback {
            offer_sequence: sequence - 1,
            admitted_to_media: true,
        });
        let tiles = actor
            .poll(&mut source, sequence, feedback, false, sequence * 100)
            .expect("poll");
        assert!(matches!(tiles, ProducerReply::Tiles { frame_id, .. } if frame_id == sequence));
        actor.published(sequence).expect("publication");
    }
    let old = request.capture_identity(1, 200, 1_700_000_000_000);
    assert!(matches!(actor.original(&old), Err(Error::Unavailable)));
    let current = request.capture_identity(9, 1000, 1_700_000_000_000);
    let mut stale = current.clone();
    stale.monotonic_timestamp_ns += 1;
    assert!(matches!(actor.original(&stale), Err(Error::Unavailable)));
    let mut wrong = current.clone();
    wrong.capture_session_id = vw_model::Id::from_parts(1_700_000_000_000, [8; 10])
        .expect("id")
        .to_string();
    assert!(matches!(actor.original(&wrong), Err(Error::Unavailable)));
    assert!(matches!(
        actor.original(&current),
        Ok(ProducerReply::Original { .. })
    ));
    actor.cancel();
    assert!(matches!(actor.original(&current), Err(Error::Cancelled)));
}
