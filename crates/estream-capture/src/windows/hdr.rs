//! Whether a monitor shows HDR, and the brightness levels the HDR → SDR mapping needs.

use estream_gpu::HdrParams;
use estream_gpu::hdr::DEFAULT_SDR_WHITE_NITS;
use windows::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL, DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
    DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO,
    DISPLAYCONFIG_SDR_WHITE_LEVEL, DISPLAYCONFIG_SOURCE_DEVICE_NAME, DisplayConfigGetDeviceInfo,
    GetDisplayConfigBufferSizes, QDC_ONLY_ACTIVE_PATHS, QueryDisplayConfig,
};
use windows::Win32::Foundation::{ERROR_SUCCESS, HWND};
use windows::Win32::Graphics::Dxgi::Common::DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020;
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, IDXGIOutput6};
use windows::Win32::Graphics::Gdi::{HMONITOR, MONITOR_DEFAULTTONEAREST, MonitorFromWindow};
use windows::core::Interface;

/// An HDR monitor's levels, in nits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HdrDisplay {
    pub sdr_white_nits: f32,
    pub peak_nits: f32,
}

impl HdrDisplay {
    pub fn params(&self) -> HdrParams {
        HdrParams::new(self.sdr_white_nits, self.peak_nits)
    }
}

/// The monitor most of a window is on.
pub fn monitor_of_window(hwnd: HWND) -> HMONITOR {
    // SAFETY: plain Win32 call; an invalid window gives the nearest (primary) monitor.
    unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) }
}

/// The monitor's HDR levels when Windows shows it in HDR, `None` for SDR (or when it can't
/// tell).
pub fn monitor_hdr(monitor: HMONITOR) -> Option<HdrDisplay> {
    let (device, peak) = hdr_output(monitor)?;
    let sdr_white_nits = sdr_white_nits(&device).unwrap_or(DEFAULT_SDR_WHITE_NITS);
    Some(HdrDisplay {
        sdr_white_nits,
        peak_nits: peak,
    })
}

/// The output showing `monitor`, if it is in HDR: its GDI device name and peak (nits).
fn hdr_output(monitor: HMONITOR) -> Option<(Vec<u16>, f32)> {
    // SAFETY: DXGI enumeration with checked results; descriptors are plain data.
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1().ok()?;
        for a in 0.. {
            let Ok(adapter) = factory.EnumAdapters1(a) else {
                break;
            };
            for o in 0.. {
                let Ok(output) = adapter.EnumOutputs(o) else {
                    break;
                };
                let Ok(output) = output.cast::<IDXGIOutput6>() else {
                    continue;
                };
                let Ok(desc) = output.GetDesc1() else {
                    continue;
                };
                if desc.Monitor != monitor {
                    continue;
                }
                if desc.ColorSpace != DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020 {
                    return None;
                }
                let name = trim_wide(&desc.DeviceName);
                return Some((name, desc.MaxLuminance));
            }
        }
        None
    }
}

/// Windows' "SDR content brightness" for a display (by GDI device name, `\\.\DISPLAY1`).
fn sdr_white_nits(device: &[u16]) -> Option<f32> {
    // SAFETY: buffers are sized by GetDisplayConfigBufferSizes; request packets carry their
    // own size and are plain data.
    unsafe {
        let (mut paths, mut modes) = (0u32, 0u32);
        if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut paths, &mut modes)
            != ERROR_SUCCESS
        {
            return None;
        }
        let mut path_info = vec![DISPLAYCONFIG_PATH_INFO::default(); paths as usize];
        let mut mode_info = vec![DISPLAYCONFIG_MODE_INFO::default(); modes as usize];
        if QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut paths,
            path_info.as_mut_ptr(),
            &mut modes,
            mode_info.as_mut_ptr(),
            None,
        ) != ERROR_SUCCESS
        {
            return None;
        }
        path_info.truncate(paths as usize);
        for path in &path_info {
            let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
                header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                    r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                    size: size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
                    adapterId: path.sourceInfo.adapterId,
                    id: path.sourceInfo.id,
                },
                ..Default::default()
            };
            if DisplayConfigGetDeviceInfo(&mut source.header) != 0
                || trim_wide(&source.viewGdiDeviceName) != device
            {
                continue;
            }
            let mut white = DISPLAYCONFIG_SDR_WHITE_LEVEL {
                header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                    r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL,
                    size: size_of::<DISPLAYCONFIG_SDR_WHITE_LEVEL>() as u32,
                    adapterId: path.targetInfo.adapterId,
                    id: path.targetInfo.id,
                },
                SDRWhiteLevel: 0,
            };
            if DisplayConfigGetDeviceInfo(&mut white.header) != 0 || white.SDRWhiteLevel == 0 {
                return None;
            }
            // In thousandths of 80 nits.
            return Some(white.SDRWhiteLevel as f32 / 1000.0 * 80.0);
        }
        None
    }
}

fn trim_wide(s: &[u16]) -> Vec<u16> {
    s.iter().copied().take_while(|c| *c != 0).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_monitor_can_be_asked() {
        // Whatever this machine has: the call must not fail or hang, and HDR levels are sane.
        for m in super::super::enumerate::monitors() {
            if let Some(hdr) = monitor_hdr(m.handle) {
                eprintln!("{}: HDR {hdr:?}", m.id);
                assert!(hdr.sdr_white_nits >= 80.0 && hdr.sdr_white_nits <= 1000.0);
            } else {
                eprintln!("{}: SDR", m.id);
            }
        }
    }
}
