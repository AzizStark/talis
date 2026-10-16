//! Informational renderer-name detection, never a capability or launch gate.

pub(crate) fn software_renderer(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    [
        "llvmpipe",
        "softpipe",
        "swrast",
        "swiftshader",
        "lavapipe",
        "software rasterizer",
    ]
    .iter()
    .any(|driver| name.contains(driver))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_known_software_renderers_without_guessing_hardware_support() {
        for renderer in [
            "llvmpipe (LLVM 20.1.8, 256 bits)",
            "SOFTPIPE",
            "Mesa swrast",
            "ANGLE (Google, Vulkan (SwiftShader Device (Subzero)), SwiftShader driver)",
            "lavapipe (LLVM 20.1.8, 256 bits)",
            "Software Rasterizer",
        ] {
            assert!(software_renderer(renderer), "{renderer}");
        }
        for renderer in [
            "",
            "Unknown renderer",
            "Mesa",
            "AMD Radeon RX 6600 (radeonsi, LLVM 20.1.8)",
            "Mesa Intel(R) UHD Graphics 630 (CFL GT2)",
            "NVIDIA GeForce RTX 3060/PCIe/SSE2",
            "SVGA3D; build: RELEASE; LLVM;",
            "virgl (Intel(R) Graphics)",
            "zink Vulkan 1.3 (AMD Radeon)",
        ] {
            assert!(!software_renderer(renderer), "{renderer}");
        }
    }
}
