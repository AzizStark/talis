use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};

#[test]
fn embedded_images_use_the_existing_decoder_without_file_or_network_access() {
    for format in [image::ImageFormat::Png, image::ImageFormat::Jpeg] {
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            6,
            2,
            image::Rgb([80, 120, 160]),
        ))
        .write_to(&mut bytes, format)
        .unwrap();
        let mime = if format == image::ImageFormat::Png {
            "PNG"
        } else {
            "JPEG"
        };
        let url = format!(
            "DATA:IMAGE/{mime};BASE64,{}",
            STANDARD.encode(bytes.get_ref())
        );
        let image = load(&url, &AtomicBool::new(false)).unwrap();
        assert_eq!(image.as_ref(), &decode(bytes.get_ref()).unwrap());
        assert!(load(&url, &AtomicBool::new(true)).is_err());
        let wrong = format!(
            "data:image/{};base64,{}",
            if mime == "PNG" { "jpeg" } else { "png" },
            STANDARD.encode(bytes.get_ref())
        );
        assert_eq!(
            load(&wrong, &AtomicBool::new(false)).unwrap().as_ref(),
            &decode(bytes.get_ref()).unwrap(),
            "supported PNG/JPEG content must load even when the MIME hint is wrong"
        );
    }
    let broken = format!(
        "data:image/png;base64,{}",
        STANDARD.encode(b"\x89PNG\r\n\x1a\n")
    );
    assert!(load(&broken, &AtomicBool::new(false)).is_err());
    let mut bytes = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image::RgbImage::new(4097, 1))
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    let oversized = format!("data:image/png;base64,{}", STANDARD.encode(bytes.get_ref()));
    assert!(load(&oversized, &AtomicBool::new(false)).is_err());
}

#[test]
fn unchanged_artwork_keys_share_inline_data() {
    let key = Key {
        player: None,
        owner: None,
        track: "one".into(),
        url: "data:image/png;base64,AAAA".into(),
    };
    let clone = key.clone();
    assert!(Arc::ptr_eq(&key.url, &clone.url));
    assert_eq!(key, clone);
    let separate = Key {
        url: key.url.as_ref().into(),
        ..key.clone()
    };
    assert!(!Arc::ptr_eq(&key.url, &separate.url));
    assert_eq!(
        key, separate,
        "identical sources from later MPRIS polls must not reload"
    );
}
#[test]
fn stopped_or_disconnected_previous_cover_clears_once_and_rejects_late_work() {
    for disconnected in [false, true] {
        let notifications = Rc::new(std::cell::Cell::new(0));
        let loader = Loader::new({
            let notifications = notifications.clone();
            move || notifications.set(notifications.get() + 1)
        });
        let a = Key {
            player: Some("test".into()),
            owner: Some(":1.2".into()),
            track: "a".into(),
            url: "file:///a.png".into(),
        };
        let b = Key {
            track: "b".into(),
            url: "file:///b.png".into(),
            ..a.clone()
        };
        let mut feed = PlaybackFeed {
            enabled: true,
            active: a.player.clone(),
            artwork_owner: a.owner.clone(),
            artwork_track: Some(a.track.clone()),
            status: crate::playback::Status::Playing,
            ..Default::default()
        };
        loader.history.borrow_mut().observe(&feed, Some(&a));
        feed.artwork_track = Some(b.track.clone());
        feed.artwork = Some(crate::playback::ArtworkSource {
            track: b.track.clone(),
            url: b.url.clone(),
        });
        let artwork = feed.artwork.clone();
        loader.history.borrow_mut().observe(&feed, Some(&b));
        let image = Arc::new(image::RgbaImage::new(2, 2));
        // Inject a decoded cover and a pending result without starting GTK
        // sources or a network/file worker in this pure regression.
        {
            let mut i = loader.current.0.borrow_mut();
            i.desired = Some(b.clone());
            i.image = Some(image.clone());
            i.cache = Some((b, image.clone()));
        }
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut i = loader.previous.0.borrow_mut();
            i.desired = Some(a.clone());
            i.image = Some(image.clone());
            i.task = Some(Task {
                key: a,
                rx,
                cancel: cancel.clone(),
            });
        }
        if disconnected {
            feed.active = None;
            feed.artwork = None;
        } else {
            feed.status = crate::playback::Status::Stopped;
        }
        loader.sync(&feed);
        assert!(loader.previous_snapshot().1.is_none());
        assert!(cancel.load(Ordering::Relaxed));
        let revision = loader.previous_snapshot().0;
        let count = notifications.get();
        for _ in 0..3 {
            loader.sync(&feed);
        }
        assert_eq!(loader.previous_snapshot().0, revision);
        assert_eq!(
            notifications.get(),
            count,
            "idle state must not schedule redraws"
        );
        tx.send(Ok(image)).unwrap();
        loader.previous.poll();
        assert!(loader.previous_snapshot().1.is_none());
        assert_eq!(loader.previous_snapshot().0, revision);
        assert!(loader.previous.0.borrow().task.is_none());
        feed.active = Some("test".into());
        feed.status = crate::playback::Status::Paused;
        feed.artwork = artwork;
        loader.sync(&feed);
        assert!(loader.previous_snapshot().1.is_none());
        assert_eq!(loader.previous_snapshot().0, revision);
        loader.stop();
    }
}
#[test]
fn urls_are_restricted_and_local_names_are_decoded() {
    match location("file:///tmp/cover%20art.png").unwrap() {
        Location::Local(path) => assert_eq!(path, PathBuf::from("/tmp/cover art.png")),
        _ => panic!(),
    }
    for url in ["https://example.org/a.png", "http://127.0.0.1/a"] {
        assert!(location(url).is_ok());
    }
    for url in [
        "file://evil.example/a",
        "ftp://example.org/a",
        "https://u:p@example.org/a",
        "data:image/png,a",
        "https://example.org/a\n",
    ] {
        assert!(location(url).is_err(), "{url}");
    }
}
#[test]
fn decode_center_crops_and_bounds_texture_and_bad_input() {
    let mut pixels = image::RgbaImage::from_pixel(6, 2, image::Rgba([255, 0, 0, 255]));
    for y in 0..2 {
        for x in 2..4 {
            pixels.put_pixel(x, y, image::Rgba([0, 255, 0, 255]));
        }
    }
    let mut bytes = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(pixels)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    let decoded = decode(bytes.get_ref()).unwrap();
    assert_eq!(decoded.dimensions(), (2, 2));
    assert!(decoded.pixels().all(|p| p.0 == [0, 255, 0, 255]));
    assert!(decode(b"not an image").is_err());
    assert!(bounded_read(Cursor::new(Vec::new())).is_err());
    assert!(bounded_read(std::io::repeat(0)).is_err());
}
#[test]
fn decoding_accepts_jpeg_downsizes_and_rejects_oversized_dimensions() {
    let mut bytes = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image::RgbImage::new(4, 2))
        .write_to(&mut bytes, image::ImageFormat::Jpeg)
        .unwrap();
    assert_eq!(decode(bytes.get_ref()).unwrap().dimensions(), (2, 2));
    for (width, height, expected) in [(1025, 1025, Some((1024, 1024))), (4097, 1, None)] {
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image::RgbImage::new(width, height))
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        match expected {
            Some(dimensions) => {
                assert_eq!(decode(bytes.get_ref()).unwrap().dimensions(), dimensions)
            }
            None => assert!(decode(bytes.get_ref()).is_err()),
        }
    }
}
#[test]
fn late_cancelled_results_and_failures_do_not_replace_the_current_cover() {
    let loader = ImageLoader::new(|| {});
    let key = Key {
        player: Some("test".into()),
        owner: None,
        track: "one".into(),
        url: "file:///one.png".into(),
    };
    let (tx, rx) = mpsc::channel();
    let image = Arc::new(image::RgbaImage::new(2, 2));
    {
        let mut i = loader.0.borrow_mut();
        i.desired = Some(key.clone());
        i.image = Some(image.clone());
        i.task = Some(Task {
            key: key.clone(),
            rx,
            cancel: Arc::new(AtomicBool::new(true)),
        });
    }
    tx.send(Ok(Arc::new(image::RgbaImage::new(4, 4)))).unwrap();
    loader.poll();
    assert_eq!(loader.snapshot().1.unwrap().dimensions(), (2, 2));
    let (tx, rx) = mpsc::channel();
    {
        let mut i = loader.0.borrow_mut();
        i.image = None;
        i.task = Some(Task {
            key,
            rx,
            cancel: Arc::new(AtomicBool::new(false)),
        });
    }
    tx.send(Err(anyhow::anyhow!("bad image"))).unwrap();
    loader.poll();
    assert!(loader.snapshot().1.is_none());
    assert!(
        loader.0.borrow().task.is_none(),
        "failures must not cause retry storms"
    );
    loader.sync_key(None, None);
    assert!(loader.0.borrow().desired.is_none());
    loader.stop();
}
