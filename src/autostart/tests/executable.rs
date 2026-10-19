use super::*;
use std::os::unix::fs::{PermissionsExt, symlink};

fn wrapper(fixture: &Fixture) -> PathBuf {
    let path = fixture.root.join("installed wrapper");
    fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[test]
fn unwrapped_launch_keeps_current_executable() {
    let inner = Fixture::new().launch().executable;
    assert_eq!(select_executable(inner.clone(), None).unwrap(), inner);
}

#[test]
fn installed_wrapper_preserves_profile_symlink() {
    let fixture = Fixture::new();
    let profile = fixture.root.join("profile-launcher");
    symlink(wrapper(&fixture), &profile).unwrap();
    assert_eq!(
        select_executable(fixture.launch().executable, Some(profile.clone())).unwrap(),
        profile
    );
}

#[test]
fn startup_entry_uses_wrapper_instead_of_inner_executable() {
    let fixture = Fixture::new();
    let mut launch = fixture.launch();
    let inner = launch.executable.clone();
    let wrapper = wrapper(&fixture);
    launch.executable = select_executable(inner.clone(), Some(wrapper.clone())).unwrap();
    fixture.manager().enable(&launch).unwrap();
    let entry = fs::read_to_string(&fixture.manager().entry).unwrap();
    assert!(entry.contains(&format!("TryExec={}", wrapper.display())));
    assert!(!entry.contains(&format!("TryExec={}", inner.display())));
}

#[test]
fn invalid_wrapper_paths_are_rejected() {
    let fixture = Fixture::new();
    let inner = fixture.launch().executable;
    for invalid in [
        PathBuf::new(),
        PathBuf::from("relative"),
        fixture.root.join("missing"),
        fixture.root.clone(),
    ] {
        assert!(
            select_executable(inner.clone(), Some(invalid.clone())).is_err(),
            "accepted invalid wrapper: {}",
            invalid.display()
        );
    }
}

#[test]
fn non_executable_wrapper_is_rejected() {
    let fixture = Fixture::new();
    let wrapper = wrapper(&fixture);
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(select_executable(fixture.launch().executable, Some(wrapper)).is_err());
}
