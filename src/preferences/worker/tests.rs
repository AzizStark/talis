use super::*;
use std::{fs, fs::File, os::fd::AsRawFd};

fn fixture() -> (crate::autostart::Fixture, Store) {
    let fixture = crate::autostart::Fixture::new();
    let store = Store::new(fixture.root.join("settings.toml")).unwrap();
    store
        .write(&Patch::from_iter([("fps".into(), 60_i64.into())]))
        .unwrap();
    (fixture, store)
}

fn hold(store: &Store) -> File {
    let file = File::options()
        .read(true)
        .write(true)
        .open(store.path().with_extension("lock"))
        .unwrap();
    assert_eq!(
        unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    file
}

#[test]
fn retry_recovers_from_a_real_lock_and_rereads_other_writers_values() {
    let (_fixture, store) = fixture();
    let mut lock = Some(hold(&store));
    let mut attempts = 0;
    let patch = Patch::from_iter([("rotation_rpm".into(), 45.0.into())]);
    retry_busy(
        || {
            attempts += 1;
            let result = store.try_write(&patch);
            if attempts == 2 {
                assert!(result.as_ref().unwrap_err().is::<LockBusy>());
                drop(lock.take());
                // Another writer's update must survive the eventual retry.
                store
                    .write(&Patch::from_iter([("fps".into(), 30_i64.into())]))
                    .unwrap();
            }
            result
        },
        Duration::from_secs(10),
    )
    .unwrap();
    assert_eq!(attempts, 3);
    assert_eq!(store.read().unwrap().rotation_rpm, Some(45.0));
    assert_eq!(store.read().unwrap().fps, Some(30));
}

#[test]
fn busy_deadline_is_terminal_and_unrelated_would_block_errors_are_not_retried() {
    let (_fixture, store) = fixture();
    let _lock = hold(&store);
    let before = fs::read(store.path()).unwrap();
    let mut attempts = 0;
    let error = retry_busy(
        || {
            attempts += 1;
            store.try_write(&Patch::from_iter([("fps".into(), 30_i64.into())]))
        },
        Duration::ZERO,
    )
    .unwrap_err();
    assert!(error.is::<LockBusy>());
    assert_eq!(attempts, 1);
    assert_eq!(fs::read(store.path()).unwrap(), before);
    attempts = 0;
    let error = retry_busy(
        || {
            attempts += 1;
            Err(std::io::Error::from(std::io::ErrorKind::WouldBlock).into())
        },
        LOCK_WAIT,
    )
    .unwrap_err();
    assert!(
        !error.is::<LockBusy>(),
        "only the typed flock failure is retryable"
    );
    assert_eq!(attempts, 1);
}

#[test]
fn invalid_settings_and_unsafe_targets_fail_without_retry_or_overwrite() {
    let (fixture, store) = fixture();
    for invalid in ["schema_version=99", "invalid = ["] {
        fs::write(store.path(), invalid).unwrap();
        let mut attempts = 0;
        assert!(
            retry_busy(
                || {
                    attempts += 1;
                    store.try_write(&Patch::from_iter([("fps".into(), 30_i64.into())]))
                },
                LOCK_WAIT
            )
            .is_err()
        );
        assert_eq!(attempts, 1);
        assert_eq!(fs::read_to_string(store.path()).unwrap(), invalid);
    }
    let target = fixture.root.join("untouched");
    fs::write(&target, "keep").unwrap();
    fs::remove_file(store.path()).unwrap();
    std::os::unix::fs::symlink(&target, store.path()).unwrap();
    let mut attempts = 0;
    assert!(
        retry_busy(
            || {
                attempts += 1;
                store.try_write(&Patch::from_iter([("fps".into(), 30_i64.into())]))
            },
            LOCK_WAIT
        )
        .is_err()
    );
    assert_eq!(attempts, 1);
    assert_eq!(fs::read_to_string(target).unwrap(), "keep");
}

#[test]
fn worker_reports_exhaustion_once_and_can_save_again_after_unlock() {
    let (_fixture, store) = fixture();
    let lock = hold(&store);
    let worker = Worker::new(store.clone()).unwrap();
    worker
        .save(1, Patch::from_iter([("fps".into(), 30_i64.into())]))
        .unwrap();
    let result = worker
        .results
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
    assert_eq!(result.0, 1);
    assert!(result.1.is_err());
    assert!(
        worker.poll().is_none(),
        "no intermediate failure or background retry loop"
    );
    assert_eq!(store.read().unwrap().fps, Some(60));
    drop(lock);
    worker
        .save(2, Patch::from_iter([("fps".into(), 30_i64.into())]))
        .unwrap();
    worker.flush().unwrap();
    assert_eq!(worker.poll().unwrap(), (2, Ok(())));
    assert_eq!(store.read().unwrap().fps, Some(30));
    assert!(worker.poll().is_none());
}

#[test]
fn ordered_edits_and_shutdown_flush_preserve_the_newest_values() {
    let (_fixture, store) = fixture();
    let lock = hold(&store);
    let worker = Worker::new(store.clone()).unwrap();
    worker
        .save(1, Patch::from_iter([("rotation_rpm".into(), 45.0.into())]))
        .unwrap();
    worker
        .save(
            2,
            Patch::from_iter([
                ("rotation_rpm".into(), 33.33.into()),
                ("surface_brightness".into(), 1.2.into()),
            ]),
        )
        .unwrap();
    assert!(worker.poll().is_none());
    drop(lock);
    worker.flush().unwrap();
    assert_eq!(worker.poll().unwrap(), (1, Ok(())));
    assert_eq!(worker.poll().unwrap(), (2, Ok(())));
    let saved = store.read().unwrap();
    assert_eq!(saved.fps, Some(60));
    assert_eq!(saved.rotation_rpm, Some(33.33));
    assert_eq!(saved.surface_brightness, Some(1.2));
    assert!(worker.poll().is_none());
}
