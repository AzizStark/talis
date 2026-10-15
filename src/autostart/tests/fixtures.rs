use super::super::*;

pub(crate) struct Fixture {
    pub root: PathBuf,
}
impl Fixture {
    pub fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "talis-startup-test-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self { root }
    }
    pub fn launch(&self) -> Launch {
        Launch {
            executable: std::env::current_exe().unwrap(),
            directory: self.root.clone(),
            config: Config::default(),
            monitor: 0,
            following: false,
            player: None,
            graphics_environment: vec![],
            wayland_library: None,
        }
    }
    pub fn manager(&self) -> Manager {
        Manager::new(self.root.clone()).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
