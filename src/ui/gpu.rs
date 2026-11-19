//! The choice of the renderer at start.
//!
//! wgpu on the low-power adapter is the default: on a laptop with two GPUs
//! that is the GPU that drives the screens. A window drawn by the other GPU
//! goes to the screen through a cross-adapter copy, and on the PC of the
//! report (NVIDIA dGPU, AMD iGPU) that copy drops pixel rows whenever the
//! client height is not a multiple of 4 (2026-11-20). OpenGL (glow) is the
//! fallback when wgpu finds no hardware adapter.
//!
//! `CANOXIDE_RENDERER=glow|wgpu` forces one (support); `WGPU_POWER_PREF`
//! (low|high|none) still overrides the power preference.

use eframe::egui_wgpu::{self, wgpu};

/// What a probe found: only the fields the decision needs.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Probe {
    pub name: String,
    pub backend: String,
    pub software: bool,
}

/// The renderer to start with, and a note for the display diagnostics.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Choice {
    pub wgpu: bool,
    /// "default", "forced by CANOXIDE_RENDERER" or "fallback: <reason>".
    pub note: String,
}

/// Is this adapter a software rasteriser (WARP, llvmpipe, ...)?
pub(crate) fn is_software(device_type: wgpu::DeviceType, name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    device_type == wgpu::DeviceType::Cpu || ["microsoft basic render driver", "llvmpipe", "softpipe", "swiftshader", "warp"].iter().any(|s| n.contains(s))
}

/// The decision from the override and the probe result (`Err`: no adapter).
pub(crate) fn decide(forced: Option<&str>, probe: impl FnOnce() -> Result<Probe, String>) -> Choice {
    match forced.map(str::to_ascii_lowercase).as_deref() {
        Some("glow") => return Choice { wgpu: false, note: "forced by CANOXIDE_RENDERER=glow".into() },
        Some("wgpu") => return Choice { wgpu: true, note: "forced by CANOXIDE_RENDERER=wgpu".into() },
        _ => {}
    }
    match probe() {
        Ok(p) if !p.software => Choice { wgpu: true, note: "default".into() },
        Ok(p) => Choice { wgpu: false, note: format!("fallback to glow: wgpu offers only the software adapter {} ({})", p.name, p.backend) },
        Err(e) => Choice { wgpu: false, note: format!("fallback to glow: {e}") },
    }
}

/// The power preference: `WGPU_POWER_PREF` if set, else low power.
pub(crate) fn power_preference() -> wgpu::PowerPreference {
    wgpu::PowerPreference::from_env().unwrap_or(wgpu::PowerPreference::LowPower)
}

/// The wgpu configuration of the app: eframe's default with our power preference.
pub(crate) fn wgpu_config() -> egui_wgpu::WgpuConfiguration {
    let setup = egui_wgpu::WgpuSetupCreateNew { power_preference: power_preference(), ..Default::default() };
    egui_wgpu::WgpuConfiguration { wgpu_setup: egui_wgpu::WgpuSetup::CreateNew(setup), ..Default::default() }
}

/// Ask wgpu for an adapter with the same instance settings and power
/// preference that eframe will use (no surface yet).
pub(crate) fn probe(config: &egui_wgpu::WgpuConfiguration) -> Result<Probe, String> {
    let egui_wgpu::WgpuSetup::CreateNew(create) = &config.wgpu_setup else {
        return Err("no wgpu setup to probe".into());
    };
    let instance = pollster::block_on(config.wgpu_setup.new_instance());
    let opts = wgpu::RequestAdapterOptions { power_preference: create.power_preference, force_fallback_adapter: false, compatible_surface: None };
    let adapter = pollster::block_on(instance.request_adapter(&opts)).ok_or_else(|| "wgpu found no adapter".to_string())?;
    let i = adapter.get_info();
    Ok(Probe { software: is_software(i.device_type, &i.name), name: i.name, backend: format!("{:?}", i.backend) })
}

/// Pick the renderer: the override, else a probe of wgpu.
pub(crate) fn choose() -> (eframe::Renderer, egui_wgpu::WgpuConfiguration, String) {
    let config = wgpu_config();
    let forced = std::env::var("CANOXIDE_RENDERER").ok().filter(|v| !v.is_empty());
    let c = decide(forced.as_deref(), || probe(&config));
    (if c.wgpu { eframe::Renderer::Wgpu } else { eframe::Renderer::Glow }, config, c.note)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hw() -> Result<Probe, String> {
        Ok(Probe { name: "Some GPU".into(), backend: "Vulkan".into(), software: false })
    }

    #[test]
    fn default_override_and_fallback() {
        assert_eq!(decide(None, hw), Choice { wgpu: true, note: "default".into() });
        let sw = || Ok(Probe { name: "Microsoft Basic Render Driver".into(), backend: "Dx12".into(), software: true });
        let c = decide(None, sw);
        assert!(!c.wgpu && c.note.contains("software adapter Microsoft Basic Render Driver"), "{c:?}");
        let c = decide(None, || Err("wgpu found no adapter".into()));
        assert!(!c.wgpu && c.note == "fallback to glow: wgpu found no adapter", "{c:?}");
        // The override wins and does not probe at all.
        assert!(!decide(Some("glow"), || panic!("probed")).wgpu);
        assert!(decide(Some("WGPU"), || panic!("probed")).wgpu);
        // An unknown value is ignored.
        assert!(decide(Some("dx9"), hw).wgpu);
    }

    #[test]
    fn software_adapters() {
        assert!(is_software(wgpu::DeviceType::Cpu, "anything"));
        assert!(is_software(wgpu::DeviceType::Other, "Microsoft Basic Render Driver"));
        assert!(is_software(wgpu::DeviceType::Other, "llvmpipe (LLVM 15.0.7, 256 bits)"));
        assert!(!is_software(wgpu::DeviceType::IntegratedGpu, "AMD Radeon(TM) 890M Graphics"));
        assert!(!is_software(wgpu::DeviceType::DiscreteGpu, "NVIDIA GeForce RTX 5070 Ti Laptop GPU"));
    }

    #[test]
    fn low_power_unless_the_env_says_otherwise() {
        if std::env::var_os("WGPU_POWER_PREF").is_none() {
            assert_eq!(power_preference(), wgpu::PowerPreference::LowPower);
            let egui_wgpu::WgpuSetup::CreateNew(c) = wgpu_config().wgpu_setup else { panic!("not CreateNew") };
            assert_eq!(c.power_preference, wgpu::PowerPreference::LowPower);
        }
    }
}
