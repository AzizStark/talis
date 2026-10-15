//! Locate read-only application data without depending on the launch directory.
use anyhow::{Context, Result, ensure};
use std::path::{Path, PathBuf};

fn contains_assets(directory: &Path) -> bool {
    directory
        .join("assets/scenes/1440p/day/beauty.png")
        .is_file()
}

fn locate(
    override_dir: Option<PathBuf>,
    executable: &Path,
    checkout: &Path,
) -> Result<Option<PathBuf>> {
    if let Some(directory) = override_dir {
        ensure!(
            directory.is_absolute(),
            "TALIS_DATA_DIR must be an absolute directory"
        );
        ensure!(
            contains_assets(&directory),
            "TALIS_DATA_DIR does not contain the bundled Talis assets"
        );
        return Ok(Some(directory));
    }
    // Installed and portable layouts both use PREFIX/lib/talis and
    // PREFIX/share/talis. Prefer the executable's own installation over a
    // developer checkout that might happen to exist on this machine.
    if let Some(prefix) = executable
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
    {
        let directory = prefix.join("share/talis");
        if contains_assets(&directory) {
            return Ok(Some(directory));
        }
    }
    Ok(contains_assets(checkout).then(|| checkout.to_path_buf()))
}

/// Explicit bundle override, executable-relative installation, then source tree.
/// Never probe the caller's working directory for application data.
pub fn data_directory() -> Result<Option<PathBuf>> {
    locate(
        std::env::var_os("TALIS_DATA_DIR").map(PathBuf::from),
        &std::env::current_exe().context("Locate Talis executable")?,
        Path::new(env!("CARGO_MANIFEST_DIR")),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(root: &Path) {
        let beauty = root.join("assets/scenes/1440p/day/beauty.png");
        std::fs::create_dir_all(beauty.parent().unwrap()).unwrap();
        std::fs::write(beauty, []).unwrap();
    }

    #[test]
    fn installed_and_relocated_data_take_precedence_over_checkout() {
        let f = crate::autostart::Fixture::new();
        let checkout = f.root.join("checkout");
        data(&checkout);
        for prefix in [f.root.join("usr"), f.root.join("bundle with 'quote/usr")] {
            let installed = prefix.join("share/talis");
            data(&installed);
            assert_eq!(
                locate(None, &prefix.join("lib/talis/talis-bin"), &checkout).unwrap(),
                Some(installed)
            );
        }
        assert_eq!(
            locate(None, Path::new("/missing/talis"), &checkout).unwrap(),
            Some(checkout)
        );
    }

    #[test]
    fn explicit_override_is_authoritative_and_invalid_values_do_not_fall_back() {
        let f = crate::autostart::Fixture::new();
        data(&f.root);
        let exe = Path::new("/missing/talis");
        assert_eq!(
            locate(Some(f.root.clone()), exe, Path::new("/missing")).unwrap(),
            Some(f.root.clone())
        );
        for invalid in [PathBuf::from("relative"), f.root.join("missing")] {
            assert!(locate(Some(invalid), exe, &f.root).is_err());
        }
        assert_eq!(locate(None, exe, Path::new("/missing")).unwrap(), None);
    }
}
