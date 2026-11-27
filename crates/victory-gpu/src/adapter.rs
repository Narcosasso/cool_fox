//! Which of the PC's GPUs the host device is made on.
//!
//! `D3D12CreateDevice(None, ..)` takes DXGI's default adapter. On a laptop
//! with hybrid graphics that is often the integrated GPU (the one wired to
//! the panel), which has no mesh shaders and a fraction of the speed, unless
//! Windows' per-app setting says otherwise. So the adapters are listed in
//! DXGI's high-performance order and the first that runs what we need
//! (mesh shaders at shader model 6.6) is taken.
//!
//! `VICTORY_ADAPTER` overrides it: a number (the position in the list the
//! log prints) or part of the adapter's name, case ignored ("rtx",
//! "radeon"). An adapter it names is taken even if it lacks something; a
//! wish that names none is logged and the automatic choice is made.

/// One adapter as the choice sees it, in DXGI's high-performance order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub(crate) name: String,
    /// The software rasterizer (WARP, Basic Render Driver).
    pub(crate) software: bool,
    /// A device could be made on it and it runs mesh shaders at 6.6.
    pub(crate) fits: bool,
    /// Dedicated video memory, bytes.
    pub(crate) memory: u64,
}

/// What `VICTORY_ADAPTER` asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Wish {
    Index(usize),
    Name(String),
}

impl Wish {
    pub(crate) fn parse(value: &str) -> Option<Wish> {
        let value = value.trim();
        if value.is_empty() {
            return None;
        }
        Some(match value.parse() {
            Ok(i) => Wish::Index(i),
            Err(_) => Wish::Name(value.to_lowercase()),
        })
    }
}

/// Why an adapter was taken, for the log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Why {
    /// `VICTORY_ADAPTER` named it.
    Asked,
    /// The first in high-performance order that fits.
    Fits,
    /// None fits; the first hardware adapter, so the run says what is missing.
    FirstHardware,
}

/// The adapter to make the device on, and whether a wish was given that
/// named none. `None`: no hardware adapter, leave it to DXGI's default.
pub(crate) fn choose(candidates: &[Candidate], wish: Option<&Wish>) -> (Option<(usize, Why)>, bool) {
    let asked = wish.and_then(|w| match w {
        Wish::Index(i) => (*i < candidates.len()).then_some(*i),
        Wish::Name(part) => candidates.iter().position(|c| c.name.to_lowercase().contains(part.as_str())),
    });
    if let Some(i) = asked {
        return (Some((i, Why::Asked)), false);
    }
    let unmatched = wish.is_some();
    let fits = candidates.iter().position(|c| !c.software && c.fits).map(|i| (i, Why::Fits));
    let first = || candidates.iter().position(|c| !c.software).map(|i| (i, Why::FirstHardware));
    (fits.or_else(first), unmatched)
}

#[cfg(windows)]
pub(crate) use host::pick;

#[cfg(windows)]
mod host {
    use super::{Candidate, Why, Wish, choose};
    use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_12_0;
    use windows::Win32::Graphics::Direct3D12::*;
    use windows::Win32::Graphics::Dxgi::*;

    /// Whether `device` runs translated mesh shaders: mesh shaders and
    /// shader model 6.6, as `Host::mesh_shaders` asks.
    fn fits(device: &ID3D12Device) -> bool {
        let mut options = D3D12_FEATURE_DATA_D3D12_OPTIONS7::default();
        let mut model = D3D12_FEATURE_DATA_SHADER_MODEL {
            HighestShaderModel: D3D_SHADER_MODEL_6_6,
        };
        // SAFETY: each query writes its own structure of the given size.
        let ok = unsafe {
            device
                .CheckFeatureSupport(D3D12_FEATURE_D3D12_OPTIONS7, (&raw mut options).cast(), size_of_val(&options) as u32)
                .is_ok()
                && device
                    .CheckFeatureSupport(D3D12_FEATURE_SHADER_MODEL, (&raw mut model).cast(), size_of_val(&model) as u32)
                    .is_ok()
        };
        ok && options.MeshShaderTier != D3D12_MESH_SHADER_TIER_NOT_SUPPORTED
            && model.HighestShaderModel.0 >= D3D_SHADER_MODEL_6_6.0
    }

    /// The adapter to make the host device on, with every adapter seen and
    /// the choice logged; `None` leaves it to DXGI's default adapter (no
    /// factory with GPU preferences, or no hardware adapter).
    pub(crate) fn pick() -> Option<IDXGIAdapter1> {
        // SAFETY: plain DXGI queries; the devices made to ask what an
        // adapter supports are dropped before the host's own is made.
        unsafe {
            let factory: IDXGIFactory6 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)).ok()?;
            let mut adapters = Vec::new();
            let mut candidates = Vec::new();
            for i in 0.. {
                let Ok(adapter) =
                    factory.EnumAdapterByGpuPreference::<IDXGIAdapter1>(i, DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE)
                else {
                    break;
                };
                let Ok(desc) = adapter.GetDesc1() else { continue };
                let end = desc.Description.iter().position(|&c| c == 0).unwrap_or(desc.Description.len());
                let software = desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0;
                let fits = !software && {
                    let mut device: Option<ID3D12Device> = None;
                    D3D12CreateDevice(&adapter, D3D_FEATURE_LEVEL_12_0, &mut device).is_ok()
                        && device.as_ref().is_some_and(fits)
                };
                let candidate = Candidate {
                    name: String::from_utf16_lossy(&desc.Description[..end]),
                    software,
                    fits,
                    memory: desc.DedicatedVideoMemory as u64,
                };
                tracing::info!(
                    index = candidates.len(),
                    name = %candidate.name,
                    vendor = format_args!("{:#06x}", desc.VendorId),
                    device = format_args!("{:#06x}", desc.DeviceId),
                    memory_mib = candidate.memory >> 20,
                    software,
                    mesh_shaders_6_6 = fits,
                    "GPU seen (high-performance order)"
                );
                adapters.push(adapter);
                candidates.push(candidate);
            }
            let wish = std::env::var("VICTORY_ADAPTER").ok().and_then(|v| Wish::parse(&v));
            let (chosen, unmatched) = choose(&candidates, wish.as_ref());
            if unmatched {
                tracing::warn!(wish = ?wish, "VICTORY_ADAPTER names no GPU seen; choosing as without it");
            }
            let (i, why) = chosen?;
            let c = &candidates[i];
            match why {
                Why::FirstHardware => tracing::warn!(
                    name = %c.name,
                    "no GPU runs mesh shaders at shader model 6.6; taking the first one, translated mesh shaders will not run"
                ),
                Why::Asked if !c.fits => tracing::warn!(
                    name = %c.name,
                    "VICTORY_ADAPTER asked for a GPU without mesh shaders at shader model 6.6"
                ),
                _ => {}
            }
            tracing::info!(index = i, name = %c.name, memory_mib = c.memory >> 20, why = ?why, "host GPU");
            adapters.into_iter().nth(i)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gpu(name: &str, software: bool, fits: bool) -> Candidate {
        Candidate { name: name.into(), software, fits, memory: 0 }
    }

    /// A hybrid laptop as DXGI lists it in high-performance order.
    fn laptop() -> Vec<Candidate> {
        vec![
            gpu("NVIDIA GeForce RTX 4060 Laptop GPU", false, true),
            gpu("Intel(R) UHD Graphics", false, false),
            gpu("Microsoft Basic Render Driver", true, false),
        ]
    }

    #[test]
    fn the_first_gpu_that_fits_is_taken() {
        assert_eq!(choose(&laptop(), None), (Some((0, Why::Fits)), false));
        // Also when DXGI puts the integrated GPU first.
        let mut swapped = laptop();
        swapped.swap(0, 1);
        assert_eq!(choose(&swapped, None), (Some((1, Why::Fits)), false));
    }

    #[test]
    fn without_one_that_fits_the_first_hardware_gpu_is_taken() {
        let list = vec![gpu("Basic Render", true, false), gpu("Old card", false, false)];
        assert_eq!(choose(&list, None), (Some((1, Why::FirstHardware)), false));
        assert_eq!(choose(&[gpu("Basic Render", true, false)], None), (None, false));
        assert_eq!(choose(&[], None), (None, false));
    }

    #[test]
    fn a_wish_wins_by_number_or_by_part_of_the_name() {
        let list = laptop();
        assert_eq!(choose(&list, Wish::parse("1").as_ref()), (Some((1, Why::Asked)), false));
        assert_eq!(choose(&list, Wish::parse(" Intel ").as_ref()), (Some((1, Why::Asked)), false));
        assert_eq!(choose(&list, Wish::parse("rtx").as_ref()), (Some((0, Why::Asked)), false));
    }

    #[test]
    fn a_wish_that_names_nothing_falls_back_and_says_so() {
        let list = laptop();
        assert_eq!(choose(&list, Wish::parse("7").as_ref()), (Some((0, Why::Fits)), true));
        assert_eq!(choose(&list, Wish::parse("radeon").as_ref()), (Some((0, Why::Fits)), true));
        assert_eq!(Wish::parse("   "), None);
    }
}
