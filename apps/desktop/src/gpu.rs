//! Graphics set-up: which API the window uses and how frames are presented.

use editor_core::prefs::{GfxBackend, GraphicsPrefs, PresentChoice};
use eframe::egui_wgpu::{self, SurfaceConfig, WgpuConfiguration, WgpuSetup, WgpuSetupCreateNew};
use eframe::wgpu;

/// The surface settings for a presentation choice.
pub fn surface(p: PresentChoice) -> SurfaceConfig {
    match p {
        PresentChoice::Smooth => SurfaceConfig::HIGH_THROUGHPUT,
        PresentChoice::LowLatency => SurfaceConfig::LOW_LATENCY,
        PresentChoice::Uncapped => SurfaceConfig {
            present_mode: wgpu::PresentMode::AutoNoVsync,
            desired_maximum_frame_latency: Some(2),
        },
    }
}

fn backends(b: GfxBackend) -> Option<wgpu::Backends> {
    match b {
        GfxBackend::Auto => None,
        GfxBackend::Dx12 => Some(wgpu::Backends::DX12),
        GfxBackend::Vulkan => Some(wgpu::Backends::VULKAN),
        GfxBackend::Gl => Some(wgpu::Backends::GL),
    }
}

/// wgpu configuration from the saved preferences. `Auto` keeps the library defaults, including
/// the `WGPU_BACKEND` environment override.
pub fn configuration(g: &GraphicsPrefs) -> WgpuConfiguration {
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    if let Some(b) = backends(g.backend) {
        setup.instance_descriptor.backends = b;
    }
    WgpuConfiguration {
        surface: surface(g.present),
        wgpu_setup: WgpuSetup::CreateNew(setup),
        ..egui_wgpu::WgpuConfiguration::default()
    }
}

/// One line describing the adapter in use, for the Preferences dialog.
pub fn describe(rs: &egui_wgpu::RenderState) -> String {
    let i = rs.adapter.get_info();
    format!("{} — {:?}, {:?}", i.name, i.backend, i.device_type)
}
