use super::*;
#[test]
fn bounded_reads_hashing_and_manifests_reject_sparse_oversized_files() {
    let fixture = crate::autostart::Fixture::new();
    let path = fixture.root.join("asset.bin");
    fs::write(&path, b"abc").unwrap();
    assert_eq!(hash_file(&path).unwrap(), sha256(b"abc"));
    assert_eq!(read_bounded(&path, 3).unwrap(), b"abc");
    assert!(
        read_bounded(&path, 2)
            .unwrap_err()
            .downcast_ref::<ResourceLimit>()
            .is_some()
    );
    let sparse = fs::OpenOptions::new().write(true).open(&path).unwrap();
    sparse.set_len(MAX_ASSET_BYTES + 1).unwrap();
    for error in [
        read_bounded(&path, MAX_ASSET_BYTES).unwrap_err(),
        hash_file(&path).unwrap_err(),
        read_manifest::<serde_json::Value>(&path).unwrap_err(),
    ] {
        assert!(error.downcast_ref::<ResourceLimit>().is_some(), "{error:#}");
    }
    let file = AssetFile {
        file: "asset.bin".into(),
        sha256: sha256(b"abc"),
    };
    assert!(
        read_hashed(&fixture.root, &file)
            .unwrap_err()
            .downcast_ref::<ResourceLimit>()
            .is_some()
    );
    // Simulate growth after the metadata check without racing a real writer.
    assert!(
        read_capped(Cursor::new(b"abcd"), 3, "growing asset", 3)
            .unwrap_err()
            .downcast_ref::<ResourceLimit>()
            .is_some()
    );
    assert!(read_bounded(&fixture.root, 100).is_err());
}

#[test]
fn scene_budget_checked_arithmetic_and_optional_rollback() {
    let mut budget = SceneBudget::default();
    budget
        .reserve(MAX_SCENE_BYTES - 4, "existing layers")
        .unwrap();
    assert!(
        budget
            .reserve(5, "new layer")
            .unwrap_err()
            .downcast_ref::<ResourceLimit>()
            .is_some()
    );
    assert_eq!(budget.bytes(), MAX_SCENE_BYTES - 4);
    let corrupt: Option<()> = budget
        .optional(
            |budget| {
                budget.reserve(4, "failed optional layer")?;
                anyhow::bail!("corrupt data")
            },
            "fixture",
        )
        .unwrap();
    assert!(corrupt.is_none());
    assert_eq!(budget.bytes(), MAX_SCENE_BYTES - 4);
    let error = budget
        .optional::<()>(
            |budget| {
                budget
                    .reserve(5, "oversized optional layer")
                    .context("optional load")
            },
            "fixture",
        )
        .unwrap_err();
    assert!(error.downcast_ref::<ResourceLimit>().is_some());
    budget.reserve(4, "last supported bytes").unwrap();
    assert!(buffer_bytes(&[u64::MAX, 2], "overflow").is_err());
    assert!(budget.reserve(u64::MAX, "overflow").is_err());
    assert!(image_bytes((u32::MAX, 1), 4, "oversized dimensions").is_err());
    assert!(image_bytes((8192, 8192), 4, "too many pixels").is_err());
    assert!(image_bytes((7680, 4320), 4, "8K beauty").is_ok());
}

/// Change only IHDR/CRC and leave the tiny compressed pixel payload alone.
/// A decode would fail; header rejection must happen before it is attempted.
fn header_fixture(w: u32, h: u32) -> Vec<u8> {
    let mut encoded = Cursor::new(Vec::new());
    image::DynamicImage::ImageLuma8(image::GrayImage::new(1, 1))
        .write_to(&mut encoded, image::ImageFormat::Png)
        .unwrap();
    let mut bytes = encoded.into_inner();
    bytes[16..20].copy_from_slice(&w.to_be_bytes());
    bytes[20..24].copy_from_slice(&h.to_be_bytes());
    let mut crc = !0_u32;
    for byte in &bytes[12..29] {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb88320 & 0_u32.wrapping_sub(crc & 1));
        }
    }
    bytes[29..33].copy_from_slice(&(!crc).to_be_bytes());
    bytes
}

#[test]
fn header_dimensions_are_checked_before_decoding_pixels() {
    let mismatch = decode_image(&header_fixture(4, 4), (1, 1), "header fixture").unwrap_err();
    assert!(
        mismatch.to_string().contains("Incorrect image dimensions"),
        "{mismatch:#}"
    );
    for dimensions in [(8193, 1), (8192, 8192)] {
        let error = decode_image(
            &header_fixture(dimensions.0, dimensions.1),
            (1, 1),
            "header fixture",
        )
        .unwrap_err();
        assert!(error.downcast_ref::<ResourceLimit>().is_some(), "{error:#}");
    }
}
#[test]
fn filenames_stay_local_to_the_package() {
    assert!(local_filename("pose-000.png"));
    for file in [
        "",
        ".",
        "..",
        "../pose.png",
        "/tmp/pose.png",
        "nested/pose.png",
    ] {
        assert!(!local_filename(file), "{file}");
    }
}
#[test]
fn sha256_format_and_known_digest() {
    let digest = sha256(b"abc");
    assert_eq!(
        digest,
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert!(valid_hash(&digest.to_uppercase()));
    assert!(!valid_hash(&"z".repeat(64)));
    assert!(!valid_hash("1234"));
}

#[test]
fn image_checks_reject_corrupt_bytes_and_wrong_dimensions() {
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageLuma8(image::GrayImage::new(3, 2))
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    let decoded = decode_image(bytes.get_ref(), (3, 2), "test mask").unwrap();
    assert_eq!(decoded.color(), image::ColorType::L8);
    assert!(decode_image(bytes.get_ref(), (2, 3), "test mask").is_err());
    assert!(decode_image(b"not a PNG", (3, 2), "test mask").is_err());
}
