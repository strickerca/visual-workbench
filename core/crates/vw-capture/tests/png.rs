use vw_capture::*;
fn temp() -> tempfile::TempDir {
    #[cfg(target_os = "android")]
    {
        tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap()
    }
    #[cfg(not(target_os = "android"))]
    {
        tempfile::tempdir().unwrap()
    }
}
#[test]
fn png_publication_preserves_exact_rgba_and_source_hash() {
    let directory = temp();
    let pixels = [1, 2, 3, 255, 250, 17, 0, 255];
    let (hash, count) = publish_png(
        directory.path(),
        2,
        1,
        &pixels,
        Limits::default(),
        &Cancellation::default(),
    )
    .unwrap();
    let bytes = std::fs::read(directory.path().join("capture.png")).unwrap();
    assert_eq!(count, bytes.len() as u64);
    assert_eq!(hash, blake3::hash(&bytes).to_hex().to_string());
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().unwrap();
    let mut output = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut output).unwrap();
    assert_eq!(&output[..info.buffer_size()], pixels);
}
#[test]
fn existing_file_and_cancelled_output_are_never_replaced() {
    let directory = temp();
    let path = directory.path().join("capture.png");
    std::fs::write(&path, b"owned by another call").unwrap();
    assert!(
        publish_png(
            directory.path(),
            1,
            1,
            &[0, 0, 0, 255],
            Limits::default(),
            &Cancellation::default()
        )
        .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"owned by another call");
    let clean = temp();
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(
        publish_png(
            clean.path(),
            1,
            1,
            &[0, 0, 0, 255],
            Limits::default(),
            &cancel
        )
        .is_err()
    );
    assert!(!clean.path().join("capture.png").exists());
}
#[test]
fn failed_encoded_limit_removes_only_its_created_file() {
    let directory = temp();
    let sentinel = directory.path().join("other");
    std::fs::write(&sentinel, b"preserve").unwrap();
    assert!(
        publish_png(
            directory.path(),
            1,
            1,
            &[0, 0, 0, 255],
            Limits {
                png_bytes: 8,
                ..Limits::default()
            },
            &Cancellation::default()
        )
        .is_err()
    );
    assert!(!directory.path().join("capture.png").exists());
    assert_eq!(std::fs::read(sentinel).unwrap(), b"preserve");
}
