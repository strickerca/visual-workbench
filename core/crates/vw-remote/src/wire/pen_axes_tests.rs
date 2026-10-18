use super::*;

#[test]
fn old_private_samples_default_to_pressure_only_and_exact_axis_masks_roundtrip() -> Result<()> {
    let json = r#"{"phase":"Down","x":1,"y":2,"pressure":512,"tilt_x":0,"tilt_y":0,"rotation":0,"pen_flags":0}"#;
    let mut sample: PenSample = serde_json::from_str(json).map_err(|_| Error::Invalid)?;
    assert_eq!(sample.pen_mask, 1);
    for mask in [1, 3, 13, 15] {
        sample.pen_mask = mask;
        sample.validate()?;
        let command = Command::Pen {
            sequence: 1,
            input_seq: 2,
            sample,
        };
        let mut bytes = Vec::new();
        write_command(&mut bytes, &command)?;
        assert!(
            matches!(read_command(&mut bytes.as_slice())?, Command::Pen {sample: value, ..} if value.pen_mask == mask)
        );
    }
    for mask in [0, 2, 4, 5, 8, 9, 16, u32::MAX] {
        sample.pen_mask = mask;
        assert!(sample.validate().is_err());
        let command = Command::Pen {
            sequence: 1,
            input_seq: 2,
            sample,
        };
        assert!(write_command(&mut Vec::new(), &command).is_err());
    }
    Ok(())
}
