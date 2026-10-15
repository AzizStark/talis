use super::*;
#[test]
fn unknown_expired_and_unlocked_screensavers_do_not_suspend() {
    let monitor = Monitor {
        providers: vec![Rc::new(Provider::default())],
        connection: RefCell::new(None),
        signals: RefCell::new(vec![]),
        task: RefCell::new(None),
        calls: RefCell::new(vec![]),
        stopped: Cell::new(false),
    };
    let now = Instant::now();
    assert!(!monitor.active(now));
    monitor.providers[0].status.set(Some((true, now)));
    assert!(monitor.active(now));
    assert!(!monitor.active(now + Duration::from_secs(7)));
    monitor.providers[0].status.set(Some((false, now)));
    assert!(!monitor.active(now));
    monitor.providers[0].status.set(Some((true, now)));
    monitor.stop();
    assert!(!monitor.active(now));
}

mod recovery;
