mod common;
use common::*;
use vw_instructions::*;
use vw_ops::HostSequencer;
use vw_proto::v1::{self, object_state::Shape, op::Kind};

#[test]
fn marker_numbers_follow_accepted_placement_and_keep_identity_and_pixel_conventions() -> TestResult
{
    let mut host = fixture()?;
    place(&mut host, 20)?;
    place(&mut host, 1)?; // Earlier UUID and wall time; still the second placement.
    let export = InstructionExport::from_view(&view(&host)?)?;
    assert_eq!(
        export
            .markers()
            .iter()
            .map(|v| v.number)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(export.markers()[0].object_id, id(30)?);
    assert_eq!(export.markers()[1].instruction_id, id(101)?);
    assert_eq!(export.markers()[0].point_document, [1.5, 2.5]);
    assert_eq!(
        export.markers()[0].bounds_document,
        Some([0.0, 2.0, 9.0, 7.0])
    );
    assert_eq!(host.project().instructions.len(), 2);
    assert_eq!(export.source().state_hash, host.project().state_hash()?);
    Ok(())
}

#[test]
fn deleting_middle_marker_renumbers_atomically_and_canonical_undo_restores_exact_state()
-> TestResult {
    let mut host = fixture()?;
    for n in 1..=3 {
        place(&mut host, n)?;
    }
    let original = host.project().state_hash()?;
    let plan = view(&host)?.delete_marker(meta(5)?, &id(12)?)?;
    assert_eq!(plan.transaction().ops.len(), 3);
    plan.submit(&mut host, &device(1), 3000)?;
    let export = InstructionExport::from_view(&view(&host)?)?;
    assert_eq!(export.markers().len(), 2);
    assert_eq!(export.markers()[1].object_id, id(13)?);
    assert_eq!(export.markers()[1].instruction_id, id(103)?);
    assert_eq!(export.markers()[1].number, 2);
    assert_eq!(
        host.project()
            .instructions
            .get(&id(103)?)
            .ok_or("instruction")?
            .definition
            .text,
        "Instruction 3"
    );
    raw(
        &mut host,
        6,
        vec![Kind::UndoTransaction(v1::UndoTransaction {
            target_txn_id: plan.transaction().txn_id.clone(),
        })],
    )?;
    assert_eq!(host.project().state_hash()?, original);
    assert_eq!(
        InstructionExport::from_view(&view(&host)?)?.markers()[2].number,
        3
    );
    Ok(())
}

#[test]
fn stale_concurrent_marker_plan_refuses_without_mutation_and_can_be_replanned() -> TestResult {
    let mut host = fixture()?;
    let base = view(&host)?;
    let first = base.place_marker(meta(1)?, placement(1)?)?;
    let stale = base.place_marker(meta(2)?, placement(2)?)?;
    first.submit(&mut host, &device(1), 3000)?;
    let before = host.checkpoint_bytes()?;
    assert!(matches!(
        stale.submit(&mut host, &device(1), 3000),
        Err(Error::Stale)
    ));
    assert_eq!(host.checkpoint_bytes()?, before);
    let refreshed = view(&host)?.place_marker(meta(3)?, placement(2)?)?;
    refreshed.submit(&mut host, &device(1), 3000)?;
    assert_eq!(
        InstructionExport::from_view(&view(&host)?)?.markers()[1].number,
        2
    );
    Ok(())
}

#[test]
fn exact_accepted_retry_survives_newer_work_and_checkpoint_restart() -> TestResult {
    let mut host = fixture()?;
    let first = place(&mut host, 1)?;
    let receipt = first.submit(&mut host, &device(1), 3000)?;
    place(&mut host, 2)?;
    let mut reopened = HostSequencer::from_checkpoint_bytes(&host.checkpoint_bytes()?)?;
    let before = reopened.checkpoint_bytes()?;
    let retry = first.submit(&mut reopened, &device(1), 4000)?;
    assert!(retry.duplicate);
    assert_eq!(retry.ack, receipt.ack);
    assert_eq!(reopened.checkpoint_bytes()?, before);
    assert!(matches!(
        first.submit(&mut reopened, &device(2), 4000),
        Err(Error::Unauthorized)
    ));
    Ok(())
}

#[test]
fn every_entry_method_preserves_literal_text_and_role_updates_do_not_change_appearance()
-> TestResult {
    let mut host = fixture()?;
    place(&mut host, 1)?;
    let style = host
        .project()
        .objects
        .get(&id(11)?)
        .ok_or("object")?
        .state
        .style;
    let literal = "  Preserve accents: café, 日本語.\n`Quoted` text.\r\n  ";
    for (i, method) in [
        EntryMethod::PcKeyboard,
        EntryMethod::PhoneKeyboard,
        EntryMethod::Voice,
        EntryMethod::Handwriting,
    ]
    .into_iter()
    .enumerate()
    {
        let v = view(&host)?;
        let mut update = edit(&v, &id(101)?, literal, method)?;
        update.role = Role::Preserve;
        let metadata = meta(10 + i as u64)?;
        let expected_time = metadata.created_at_ms;
        let plan = v.set_instruction(metadata, update)?;
        plan.submit(&mut host, &device(1), 4000)?;
        let stored = host
            .project()
            .instructions
            .get(&id(101)?)
            .ok_or("instruction")?;
        assert_eq!(stored.definition.text, literal);
        assert_eq!(stored.definition.entry_method, method.as_str());
        assert_eq!(stored.updated_at_ms, expected_time);
        let object = host.project().objects.get(&id(11)?).ok_or("object")?;
        assert_eq!(object.state.role, v1::Role::Preserve as i32);
        assert_eq!(object.state.style, style);
    }
    Ok(())
}

#[test]
fn one_global_instruction_and_no_duplicate_or_stranded_marker_links() -> TestResult {
    let mut host = fixture()?;
    place(&mut host, 1)?;
    let global = InstructionEdit {
        instruction_id: id(500)?,
        targets: vec![],
        role: Role::Explain,
        text: "Global literal".into(),
        entry_method: EntryMethod::PcKeyboard,
        language: "und".into(),
    };
    view(&host)?
        .set_instruction(meta(2)?, global.clone())?
        .submit(&mut host, &device(1), 4000)?;
    let mut duplicate = global;
    duplicate.instruction_id = id(501)?;
    assert!(matches!(
        view(&host)?.set_instruction(meta(3)?, duplicate),
        Err(Error::Reconciliation)
    ));
    let v = view(&host)?;
    let mut relink = edit(&v, &id(101)?, "move", EntryMethod::PcKeyboard)?;
    relink.targets.clear();
    assert!(matches!(
        v.set_instruction(meta(4)?, relink),
        Err(Error::Reconciliation)
    ));
    assert!(matches!(
        v.delete_instruction(meta(5)?, &id(101)?),
        Err(Error::Reconciliation)
    ));
    let mut conflict = edit(&v, &id(101)?, "duplicate", EntryMethod::Voice)?;
    conflict.instruction_id = id(502)?;
    assert!(matches!(
        v.set_instruction(meta(6)?, conflict),
        Err(Error::Reconciliation)
    ));
    assert_eq!(
        InstructionExport::from_view(&v)?
            .global()
            .ok_or("global")?
            .text,
        "Global literal"
    );
    Ok(())
}

#[test]
fn locked_renumbering_and_invalid_text_fail_without_consuming_state() -> TestResult {
    let mut host = fixture()?;
    place(&mut host, 1)?;
    place(&mut host, 2)?;
    let mut project = host.project().clone();
    project
        .objects
        .get_mut(&id(12)?)
        .ok_or("object")?
        .state
        .locked = true;
    let host = HostSequencer::new(project, device(1))?;
    let v = view(&host)?;
    assert!(matches!(
        v.delete_marker(meta(3)?, &id(11)?),
        Err(Error::Locked)
    ));
    assert!(matches!(
        v.set_instruction(meta(4)?, edit(&v, &id(102)?, "text", EntryMethod::Voice)?),
        Err(Error::Locked)
    ));
    let invalid = edit(&v, &id(101)?, "nul\0not allowed", EntryMethod::PcKeyboard)?;
    assert!(matches!(
        v.set_instruction(meta(5)?, invalid),
        Err(Error::Invalid(_))
    ));
    let oversized = edit(
        &v,
        &id(101)?,
        &"x".repeat(MAX_TEXT_BYTES + 1),
        EntryMethod::PcKeyboard,
    )?;
    assert!(matches!(
        v.set_instruction(meta(6)?, oversized),
        Err(Error::Limit(_))
    ));
    let mut targets = edit(&v, &id(101)?, "bounded", EntryMethod::PcKeyboard)?;
    targets.targets = vec![id(11)?; MAX_TARGETS + 1];
    assert!(matches!(
        v.set_instruction(meta(6)?, targets),
        Err(Error::Limit("instruction targets"))
    ));
    let mut exhausted = meta(7)?;
    exhausted.first_lamport = u64::MAX;
    assert!(matches!(
        v.set_instruction(
            exhausted,
            edit(&v, &id(101)?, "valid", EntryMethod::PcKeyboard)?
        ),
        Err(Error::Exhausted)
    ));
    assert_eq!(host.project().objects.len(), 2);
    Ok(())
}

#[test]
fn dangling_references_are_reported_and_never_silently_exported_or_removed() -> TestResult {
    let mut host = fixture()?;
    place(&mut host, 1)?;
    raw(
        &mut host,
        2,
        vec![Kind::DeleteObject(v1::DeleteObject {
            object_id: Some(id(11)?.to_proto()),
        })],
    )?;
    let before = host.project().canonical_bytes()?;
    let v = view(&host)?;
    assert_eq!(v.detached_instruction_ids()?, vec![id(101)?]);
    assert!(matches!(
        InstructionExport::from_view(&v),
        Err(Error::Detached)
    ));
    assert_eq!(host.project().canonical_bytes()?, before);
    v.delete_instruction(meta(3)?, &id(101)?)?
        .submit(&mut host, &device(1), 4000)?;
    assert!(
        InstructionExport::from_view(&view(&host)?)?
            .instructions()
            .is_empty()
    );
    Ok(())
}

#[test]
fn retargeting_cannot_remove_a_locked_object_or_layer_instruction_binding() -> TestResult {
    let mut seed = fixture()?;
    place(&mut seed, 1)?;
    place(&mut seed, 2)?;
    for layer_lock in [false, true] {
        let mut project = seed.project().clone();
        project.instructions.remove(&id(102)?);
        let mut unlocked_layer = project.layers.get(&id(3)?).ok_or("layer")?.clone();
        unlocked_layer.definition.layer_id = Some(id(4)?.to_proto());
        unlocked_layer.definition.order_key = "W".into();
        project.layers.insert(id(4)?, unlocked_layer);
        for number in [11, 12] {
            let object = project.objects.get_mut(&id(number)?).ok_or("object")?;
            object.state.shape = Some(Shape::Rect(v1::RectD {
                x: 1.0,
                y: 1.0,
                w: 10.0,
                h: 10.0,
            }));
            if number == 12 {
                object.state.layer_id = Some(id(4)?.to_proto());
            }
        }
        if layer_lock {
            project.layers.get_mut(&id(3)?).ok_or("layer")?.locked = true;
        } else {
            project
                .objects
                .get_mut(&id(11)?)
                .ok_or("object")?
                .state
                .locked = true;
        }
        let host = HostSequencer::new(project, device(1))?;
        let before = host.checkpoint_bytes()?;
        for targets in [vec![], vec![id(12)?]] {
            let view = view(&host)?;
            let mut update = edit(&view, &id(101)?, "Retarget", EntryMethod::PcKeyboard)?;
            update.targets = targets;
            assert!(matches!(
                view.set_instruction(meta(3)?, update),
                Err(Error::Locked)
            ));
            assert_eq!(host.checkpoint_bytes()?, before);
        }
    }
    Ok(())
}

#[test]
fn malformed_existing_numbering_requires_explicit_reconciliation() -> TestResult {
    let mut host = fixture()?;
    place(&mut host, 1)?;
    let mut project = host.project().clone();
    let shape = &mut project
        .objects
        .get_mut(&id(11)?)
        .ok_or("object")?
        .state
        .shape;
    if let Some(Shape::Marker(marker)) = shape {
        marker.number = 3;
    } else {
        return Err("marker".into());
    }
    let host = HostSequencer::new(project, device(1))?;
    let v = view(&host)?;
    assert!(matches!(
        v.place_marker(meta(2)?, placement(2)?),
        Err(Error::Reconciliation)
    ));
    assert!(matches!(
        InstructionExport::from_view(&v),
        Err(Error::Reconciliation)
    ));
    Ok(())
}
