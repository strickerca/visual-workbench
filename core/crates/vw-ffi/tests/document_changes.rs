//! `document_snapshot_changes` must describe exactly the document that
//! `document_snapshot` returns, through the calls the apps make, while never
//! rebuilding or resending a stroke the caller still holds unchanged.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod support;
use std::{collections::BTreeMap, sync::Arc};
use support::*;
use vw_core::*;
use vw_model::Id;

const BUDGET: u64 = 256 * 1024 * 1024;

fn wide_id(kind: u8, number: u32) -> String {
    let mut entropy = [kind; 10];
    entropy[6..].copy_from_slice(&number.to_be_bytes());
    Id::from_parts(TIME as u64, entropy).unwrap().to_string()
}
async fn lamport(project: &Arc<ProjectSession>) -> u64 {
    project.info().await.unwrap().next_lamport
}
async fn draw_stroke(project: &Arc<ProjectSession>, number: u32) -> String {
    let mut options = stroke();
    options.gesture_id = wide_id(0x41, number);
    options.transaction_id = wide_id(0x42, number);
    options.object_id = wide_id(0x60, number);
    options.lamport = lamport(project).await;
    options.created_at_ms = TIME + 10 + i64::from(number);
    let gesture = project.clone().begin_stroke(options.clone()).await.unwrap();
    let mut samples = batch();
    for x in &mut samples.x {
        *x += f64::from(number);
    }
    gesture.append_samples(samples).await.unwrap();
    gesture.commit(cancel()).await.unwrap();
    options.object_id
}
async fn edit_once(project: &Arc<ProjectSession>, number: u32, commands: Vec<EditCommand>) {
    let options = EditOptions {
        transaction_id: wide_id(0x43, number),
        document_id: id(2),
        device_id: device(),
        lamport: lamport(project).await,
        created_at_ms: TIME + 100 + i64::from(number),
    };
    project
        .apply_edit(options, commands, cancel())
        .await
        .unwrap();
}
fn style() -> ObjectStyle {
    ObjectStyle {
        rgba: 0x203040ff,
        width: 2.0,
        screen_constant_width: false,
        fill: None,
    }
}
fn identity() -> Transform {
    Transform {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    }
}

/// The caller's side: what it holds per stroke, and how it rebuilds the list.
#[derive(Default)]
struct Held {
    strokes: BTreeMap<String, (Vec<u8>, RenderItem)>,
    costs: BTreeMap<String, u64>,
}
impl Held {
    fn known(&self) -> Vec<KnownStroke> {
        self.strokes
            .iter()
            .map(|(object_id, (digest, _))| KnownStroke {
                object_id: object_id.clone(),
                digest: digest.clone(),
                cost: self.costs[object_id],
            })
            .collect()
    }
    /// Returns the rebuilt list and the ids that came back as kept.
    fn adopt(&mut self, changes: &DocumentSnapshotChanges) -> (Vec<RenderItem>, Vec<String>) {
        let mut items = Vec::new();
        let mut kept = Vec::new();
        let mut next = BTreeMap::new();
        let mut costs = BTreeMap::new();
        for change in &changes.items {
            let item = match &change.item {
                Some(item) => {
                    assert_eq!(item.object_id, change.object_id);
                    assert_eq!(item.layer_opacity, change.layer_opacity);
                    assert_eq!(item.layer_blend, change.layer_blend);
                    assert_eq!(item.locked, change.locked);
                    item.clone()
                }
                None => {
                    let digest = change.digest.as_ref().expect("kept without a digest");
                    let (held, item) = self
                        .strokes
                        .get(&change.object_id)
                        .expect("kept a stroke the caller never held");
                    assert_eq!(held, digest, "kept under a digest the caller did not send");
                    assert_eq!(
                        change.cost, self.costs[&change.object_id],
                        "a kept stroke counts what it counted when it was sent"
                    );
                    kept.push(change.object_id.clone());
                    let mut item = item.clone();
                    item.layer_opacity = change.layer_opacity;
                    item.layer_blend = change.layer_blend.clone();
                    item.locked = change.locked;
                    item
                }
            };
            let is_stroke = matches!(item.shape, DrawShape::Stroke { .. });
            assert_eq!(
                change.digest.is_some(),
                is_stroke,
                "only strokes carry a digest"
            );
            assert!(
                change.cost >= 1024,
                "every entry counts at least its fixed part"
            );
            if let Some(digest) = &change.digest {
                assert_eq!(digest.len(), 32);
                next.insert(change.object_id.clone(), (digest.clone(), item.clone()));
                costs.insert(change.object_id.clone(), change.cost);
            }
            items.push(item);
        }
        self.strokes = next;
        self.costs = costs;
        (items, kept)
    }
}

/// Reads the document both ways and requires the same result, field for field.
async fn same_as_full(project: &Arc<ProjectSession>, held: &mut Held) -> Vec<String> {
    let changes = project
        .document_snapshot_changes(id(2), held.known(), cancel())
        .await
        .unwrap();
    let full = project.document_snapshot(id(2), cancel()).await.unwrap();
    let (items, kept) = held.adopt(&changes);
    assert_eq!(format!("{items:?}"), format!("{:?}", full.render.items));
    assert_eq!(changes.revision, full.render.revision);
    assert_eq!(changes.document_id, full.document_id);
    assert_eq!(changes.title, full.title);
    assert_eq!(
        (changes.width, changes.height, changes.bit_depth),
        (full.width, full.height, full.bit_depth)
    );
    assert_eq!(changes.has_tone_adjustments, full.has_tone_adjustments);
    assert_eq!(
        format!("{:?}", changes.layers),
        format!("{:?}", full.layers)
    );
    kept
}

#[tokio::test]
async fn changes_rebuild_the_full_snapshot_and_keep_only_untouched_strokes() {
    let dir = tempfile::tempdir().unwrap();
    let project = create_image_project(create(&dir.path().join("project")), cancel())
        .await
        .unwrap();
    let mut held = Held::default();
    // An empty document, then three strokes and two shapes.
    assert!(same_as_full(&project, &mut held).await.is_empty());
    let strokes = [
        draw_stroke(&project, 1).await,
        draw_stroke(&project, 2).await,
        draw_stroke(&project, 3).await,
    ];
    edit_once(
        &project,
        1,
        vec![
            EditCommand::Create {
                object_id: wide_id(0x50, 1),
                layer_id: id(3),
                shape: NewShape::Rectangle {
                    rectangle: QueryRect {
                        x: 4.0,
                        y: 5.0,
                        width: 20.0,
                        height: 10.0,
                    },
                },
                style: style(),
                transform: identity(),
            },
            EditCommand::Create {
                object_id: wide_id(0x50, 2),
                layer_id: id(3),
                shape: NewShape::Text {
                    anchor: Point { x: 8.0, y: 30.0 },
                    text: "ink".into(),
                    font: "Inter".into(),
                    size: 12.0,
                },
                style: style(),
                transform: identity(),
            },
        ],
    )
    .await;
    // Nothing held yet: everything comes back complete.
    assert!(same_as_full(&project, &mut held).await.is_empty());
    assert_eq!(held.strokes.len(), 3);
    // Nothing changed: all three strokes are kept, the shapes are resent.
    assert_eq!(same_as_full(&project, &mut held).await, strokes.to_vec());

    // A fourth stroke: only it is built.
    let fourth = draw_stroke(&project, 4).await;
    assert_eq!(same_as_full(&project, &mut held).await, strokes.to_vec());
    assert!(held.strokes.contains_key(&fourth));

    // Moving one stroke changes its stored state, so it comes back complete.
    edit_once(
        &project,
        2,
        vec![EditCommand::Transform {
            object_id: strokes[1].clone(),
            transform: Transform {
                e: 7.0,
                f: -3.0,
                ..identity()
            },
        }],
    )
    .await;
    let before = held.strokes[&strokes[1]].0.clone();
    assert_eq!(
        same_as_full(&project, &mut held).await,
        vec![strokes[0].clone(), strokes[2].clone(), fourth.clone()]
    );
    assert_ne!(held.strokes[&strokes[1]].0, before);

    // Recolouring another one likewise.
    edit_once(
        &project,
        3,
        vec![EditCommand::SetStyle {
            object_id: strokes[2].clone(),
            style: ObjectStyle {
                rgba: 0xff0000ff,
                ..style()
            },
        }],
    )
    .await;
    assert_eq!(
        same_as_full(&project, &mut held).await,
        vec![strokes[0].clone(), strokes[1].clone(), fourth.clone()]
    );

    // Deleting a stroke removes it; undo brings it back byte-identical, and
    // the caller no longer holds it, so it is rebuilt once.
    edit_once(
        &project,
        4,
        vec![EditCommand::Delete {
            object_id: strokes[0].clone(),
        }],
    )
    .await;
    assert_eq!(
        same_as_full(&project, &mut held).await,
        vec![strokes[1].clone(), strokes[2].clone(), fourth.clone()]
    );
    assert!(!held.strokes.contains_key(&strokes[0]));
    let undo = EditOptions {
        transaction_id: wide_id(0x44, 1),
        document_id: id(2),
        device_id: device(),
        lamport: lamport(&project).await,
        created_at_ms: TIME + 500,
    };
    project.undo_redo(undo, false, cancel()).await.unwrap();
    assert_eq!(
        same_as_full(&project, &mut held).await,
        vec![strokes[1].clone(), strokes[2].clone(), fourth.clone()]
    );
    assert!(held.strokes.contains_key(&strokes[0]));
    assert_eq!(same_as_full(&project, &mut held).await.len(), 4);
    project.close().await.unwrap();
}

#[tokio::test]
async fn a_kept_stroke_still_reports_its_layers_current_opacity_and_lock() {
    let dir = tempfile::tempdir().unwrap();
    let project = create_image_project(create(&dir.path().join("project")), cancel())
        .await
        .unwrap();
    let mut held = Held::default();
    let first = draw_stroke(&project, 1).await;
    let second = draw_stroke(&project, 2).await;
    same_as_full(&project, &mut held).await;
    for (n, update) in [
        (
            1_u8,
            LayerControlUpdate {
                layer_id: id(3),
                name: None,
                visible: None,
                locked: None,
                opacity: Some(0.5),
                blend: None,
            },
        ),
        (
            2_u8,
            LayerControlUpdate {
                layer_id: id(3),
                name: None,
                visible: None,
                locked: Some(true),
                opacity: None,
                blend: None,
            },
        ),
    ] {
        let view = project
            .document_controls_snapshot(id(2), BUDGET, cancel())
            .await
            .unwrap();
        let metadata = WorkflowMetadata {
            transaction_id: wide_id(0x45, u32::from(n)),
            device_id: view.revision.device_id.clone(),
            first_lamport: view.revision.next_lamport,
            created_at_ms: TIME + 600 + i64::from(n),
        };
        let plan = project
            .clone()
            .prepare_document_control(
                view.binding,
                metadata,
                DocumentControlCommand::UpdateLayer { update },
                BUDGET,
                cancel(),
            )
            .await
            .unwrap();
        plan.commit(cancel()).await.unwrap();
        plan.dispose();
        // The strokes themselves did not change: both are kept, and the
        // rebuilt list still equals the full snapshot with the new layer state.
        assert_eq!(
            same_as_full(&project, &mut held).await,
            vec![first.clone(), second.clone()]
        );
    }
    let (_, item) = &held.strokes[&first];
    assert_eq!(item.layer_opacity, 0.5);
    assert!(item.locked);
    project.close().await.unwrap();
}

#[tokio::test]
async fn a_wrong_or_malformed_digest_never_keeps_a_stroke() {
    let dir = tempfile::tempdir().unwrap();
    let project = create_image_project(create(&dir.path().join("project")), cancel())
        .await
        .unwrap();
    let mut held = Held::default();
    let first = draw_stroke(&project, 1).await;
    same_as_full(&project, &mut held).await;
    let mut wrong = held.known();
    wrong[0].digest[0] ^= 1;
    let changes = project
        .document_snapshot_changes(id(2), wrong, cancel())
        .await
        .unwrap();
    assert!(
        changes.items.iter().all(|change| change.item.is_some()),
        "a stroke was kept under a digest that does not match it"
    );
    assert_eq!(
        changes.items[0].digest,
        Some(held.strokes[&first].0.clone())
    );
    // A digest for a stroke that is not in the document is simply unused.
    let mut stranger = held.known();
    stranger.push(KnownStroke {
        object_id: wide_id(0x60, 99),
        digest: vec![7; 32],
        cost: 4096,
    });
    assert_eq!(
        project
            .document_snapshot_changes(id(2), stranger, cancel())
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    for bad in [
        vec![KnownStroke {
            object_id: first.clone(),
            digest: vec![0; 31],
            cost: 4096,
        }],
        vec![KnownStroke {
            object_id: "not-an-id".into(),
            digest: vec![0; 32],
            cost: 4096,
        }],
        vec![
            KnownStroke {
                object_id: first.clone(),
                digest: vec![0; 32],
                cost: 4096,
            },
            KnownStroke {
                object_id: first.clone(),
                digest: vec![1; 32],
                cost: 4096,
            },
        ],
        // A cost no item can have: below its fixed part, or above a whole response.
        vec![KnownStroke {
            object_id: first.clone(),
            digest: vec![0; 32],
            cost: 1023,
        }],
        vec![KnownStroke {
            object_id: first.clone(),
            digest: vec![0; 32],
            cost: 64 * 1024 * 1024 + 1,
        }],
    ] {
        assert!(matches!(
            project
                .document_snapshot_changes(id(2), bad, cancel())
                .await,
            Err(CoreError::Invalid)
        ));
    }
    project.close().await.unwrap();
}

/// A kept stroke is not sent again, but the caller still holds it. It counts
/// toward the size limit exactly what it counted when it was sent, so the
/// changes read refuses a document at the same point a complete read does and
/// cannot let it grow past that.
#[tokio::test]
async fn kept_strokes_count_toward_the_same_size_limit_as_a_complete_read() {
    let dir = tempfile::tempdir().unwrap();
    let project = create_image_project(create(&dir.path().join("project")), cancel())
        .await
        .unwrap();
    let mut held = Held::default();
    for number in 0..3 {
        draw_stroke(&project, number).await;
    }
    same_as_full(&project, &mut held).await;
    // Every stroke kept: the answer still accounts for all of them in full.
    let kept = project
        .document_snapshot_changes(id(2), held.known(), cancel())
        .await
        .unwrap();
    assert!(kept.items.iter().all(|change| change.item.is_none()));
    let full = project.document_snapshot(id(2), cancel()).await.unwrap();
    let complete: u64 = kept.items.iter().map(|change| change.cost).sum();
    let sent_once: u64 = held.costs.values().sum();
    assert_eq!(complete, sent_once);
    assert!(complete >= full.render.items.len() as u64 * 1024);
    // The caller says its strokes fill the limit between them: refused with
    // the error a complete read gives for a document that large.
    let mut heavy = held.known();
    for stroke in &mut heavy {
        stroke.cost = 30 * 1024 * 1024;
    }
    assert!(matches!(
        project
            .document_snapshot_changes(id(2), heavy, cancel())
            .await,
        Err(CoreError::Backpressure)
    ));
    // Two of them fit, as they would in a complete read.
    let mut two = held.known();
    two.truncate(2);
    for stroke in &mut two {
        stroke.cost = 30 * 1024 * 1024;
    }
    let fits = project
        .document_snapshot_changes(id(2), two, cancel())
        .await
        .unwrap();
    assert_eq!(fits.items.iter().filter(|c| c.item.is_none()).count(), 2);
    project.close().await.unwrap();
}

/// The point of the call: with every stroke held, a refresh no longer costs
/// what the whole document costs. Counted, not timed: the answer carries no
/// stroke outline at all.
#[tokio::test]
async fn a_refresh_with_every_stroke_held_resends_no_outline() {
    let dir = tempfile::tempdir().unwrap();
    let project = create_image_project(create(&dir.path().join("project")), cancel())
        .await
        .unwrap();
    let mut held = Held::default();
    for number in 0..40 {
        draw_stroke(&project, number).await;
    }
    same_as_full(&project, &mut held).await;
    let full: usize = project
        .document_snapshot(id(2), cancel())
        .await
        .unwrap()
        .render
        .items
        .iter()
        .map(|item| item.stroke_contours.x.len())
        .sum();
    assert!(full > 40 * 3 * 12, "fixture strokes carry no outline");
    let newest = draw_stroke(&project, 40).await;
    let changes = project
        .document_snapshot_changes(id(2), held.known(), cancel())
        .await
        .unwrap();
    let sent: Vec<&RenderItem> = changes
        .items
        .iter()
        .filter_map(|c| c.item.as_ref())
        .collect();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].object_id, newest);
    assert_eq!(changes.items.len(), 41);
    project.close().await.unwrap();
}
