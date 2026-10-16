use super::*;

fn source(root: &Path) -> PathBuf {
    let source = root.join("system").join(UUID);
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(
        source.join("metadata.json"),
        include_str!("../../../../../integrations/gnome/metadata.json"),
    )
    .unwrap();
    std::fs::write(source.join("extension.js"), "original code").unwrap();
    std::fs::write(source.join("scene.json"), "original scene").unwrap();
    source
}

#[test]
fn shell_versions_are_identifiers_not_an_allowlist() {
    for (version, key) in [
        ("46.0", "46"),
        ("48", "48"),
        ("49.1", "49"),
        ("50.beta", "50"),
        ("51.0", "51"),
        ("99.alpha", "99"),
        ("3.38.6", "3.38"),
    ] {
        assert_eq!(version_key(version).unwrap(), key);
    }
    for invalid in ["", "GNOME 51", "../51", "3", "3.x"] {
        assert!(version_key(invalid).is_err());
    }
}

#[test]
fn system_registration_is_local_atomic_and_tracks_package_updates() {
    let fixture = crate::autostart::Fixture::new();
    let source = source(&fixture.root);
    let target = fixture.root.join("user/extensions").join(UUID);
    let original = std::fs::read(source.join("metadata.json")).unwrap();
    assert!(!registered(&source, "51").unwrap());
    assert!(!target.exists());
    prepare(&source, &target, "51.0").unwrap();
    assert!(registered(&target, "51").unwrap());
    assert!(registered(&target, "48").unwrap());
    assert_eq!(
        std::fs::read(source.join("metadata.json")).unwrap(),
        original
    );
    let entry = std::fs::read_to_string(target.join("extension.js")).unwrap();
    assert_eq!(
        entry,
        format!(
            "export {{default}} from {};\n",
            serde_json::to_string(
                gio::File::for_path(source.join("extension.js"))
                    .uri()
                    .as_str()
            )
            .unwrap()
        )
    );
    std::fs::write(source.join("extension.js"), "updated package code").unwrap();
    assert_eq!(
        std::fs::read_to_string(target.join("extension.js")).unwrap(),
        entry,
        "the entry imports the package URI instead of copying its code"
    );
    // A newly observed Shell gets registered, not blocked by the previous one.
    prepare(&target, &target, "99.0").unwrap();
    prepare(&target, &target, "99.0").unwrap();
    let value = read(&target).unwrap().0;
    assert_eq!(
        value["shell-version"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v.as_str() == Some("99"))
            .count(),
        1
    );
    assert_eq!(
        std::fs::read_to_string(target.join("scene.json")).unwrap(),
        "original scene"
    );
    let existing = std::fs::read(target.join("metadata.json")).unwrap();
    assert!(
        prepare(&source, &target, "52").is_err(),
        "never replace another existing directory"
    );
    assert_eq!(
        std::fs::read(target.join("metadata.json")).unwrap(),
        existing
    );
    assert!(
        std::fs::read_dir(target.parent().unwrap())
            .unwrap()
            .all(|e| !e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".talis-setup-"))
    );
}

#[test]
fn user_metadata_preserves_code_config_and_other_metadata() {
    let fixture = crate::autostart::Fixture::new();
    let source = source(&fixture.root);
    let mut metadata = read(&source).unwrap().0;
    metadata["user-note"] = "keep this".into();
    std::fs::write(
        source.join("metadata.json"),
        serde_json::to_vec(&metadata).unwrap(),
    )
    .unwrap();
    prepare(&source, &source, "51.0").unwrap();
    assert_eq!(read(&source).unwrap().0["user-note"], "keep this");
    assert_eq!(
        std::fs::read_to_string(source.join("extension.js")).unwrap(),
        "original code"
    );
    assert_eq!(
        std::fs::read_to_string(source.join("scene.json")).unwrap(),
        "original scene"
    );
    assert!(registered(&source, "51").unwrap());
    std::fs::write(
        source.join("metadata.json"),
        r#"{"uuid":"other","shell-version":["51"]}"#,
    )
    .unwrap();
    assert!(prepare(&source, &source, "51").is_err());
    std::fs::write(source.join("metadata.json"), " ".repeat(32769)).unwrap();
    assert!(registered(&source, "51").is_err());
}

#[test]
fn linked_source_files_are_not_modified() {
    let fixture = crate::autostart::Fixture::new();
    let source = source(&fixture.root);
    let original = std::fs::read(source.join("metadata.json")).unwrap();
    let user = fixture.root.join("user").join(UUID);
    std::fs::create_dir_all(user.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&source, &user).unwrap();
    assert!(prepare(&user, &user, "51").is_err());
    assert_eq!(
        std::fs::read(source.join("metadata.json")).unwrap(),
        original
    );
    std::fs::remove_file(&user).unwrap();
    std::fs::create_dir(&user).unwrap();
    std::os::unix::fs::symlink(source.join("metadata.json"), user.join("metadata.json")).unwrap();
    prepare(&user, &user, "51").unwrap();
    assert!(registered(&user, "51").unwrap());
    assert!(!user.join("metadata.json").is_symlink());
    assert_eq!(
        std::fs::read(source.join("metadata.json")).unwrap(),
        original
    );
}
