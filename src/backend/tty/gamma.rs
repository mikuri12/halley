use std::num::NonZeroU64;
use std::os::fd::AsFd;

use smithay::backend::drm::DrmDevice;
use smithay::reexports::drm::control::{Device as ControlDevice, crtc, property};

#[repr(C)]
#[derive(Clone, Copy)]
struct DrmColorLut {
    red: u16,
    green: u16,
    blue: u16,
    reserved: u16,
}

#[derive(Debug)]
struct AtomicGamma {
    crtc: crtc::Handle,
    lut: property::Handle,
    size: property::Handle,
    previous_blob: Option<NonZeroU64>,
}

#[derive(Debug)]
pub(super) struct GammaState {
    crtc: crtc::Handle,
    atomic: Option<AtomicGamma>,
    current: Option<Vec<u16>>,
    pending: bool,
}

impl GammaState {
    pub fn new(device: &DrmDevice, crtc: crtc::Handle) -> Self {
        Self {
            crtc,
            atomic: AtomicGamma::new(device, crtc).ok(),
            current: None,
            pending: false,
        }
    }

    pub fn size(&self, device: &DrmDevice) -> Result<u32, String> {
        if let Some(atomic) = &self.atomic {
            return atomic.size(device);
        }
        let size = device
            .get_crtc(self.crtc)
            .map_err(|err| format!("failed to query CRTC gamma size: {err}"))?
            .gamma_length();
        if size == 0 {
            Err("output does not support gamma ramps".into())
        } else {
            Ok(size)
        }
    }

    pub fn set(
        &mut self,
        device: &DrmDevice,
        ramp: Option<Vec<u16>>,
        active: bool,
    ) -> Result<(), String> {
        if let Some(ramp) = ramp.as_ref() {
            let expected = self.size(device)? as usize * 3;
            if ramp.len() != expected {
                return Err(format!(
                    "gamma ramp has {} entries, expected {expected}",
                    ramp.len()
                ));
            }
        }
        self.current = ramp;
        if !active {
            self.pending = true;
            return Ok(());
        }
        self.apply_current(device)?;
        self.pending = false;
        Ok(())
    }

    pub fn restore_after_resume(&mut self, device: &DrmDevice) -> Result<(), String> {
        self.apply_current(device)?;
        self.pending = false;
        Ok(())
    }

    fn apply_current(&mut self, device: &DrmDevice) -> Result<(), String> {
        if let Some(atomic) = &mut self.atomic {
            atomic.set(device, self.current.as_deref())
        } else {
            set_legacy(device, self.crtc, self.current.as_deref())
        }
    }
}

impl AtomicGamma {
    fn new(device: &DrmDevice, crtc: crtc::Handle) -> Result<Self, String> {
        let mut lut = None;
        let mut size = None;
        let properties = device
            .get_properties(crtc)
            .map_err(|err| format!("failed to query CRTC properties: {err}"))?;
        for (handle, _) in properties {
            let Ok(info) = device.get_property(handle) else {
                continue;
            };
            match info.name().to_bytes() {
                b"GAMMA_LUT" if matches!(info.value_type(), property::ValueType::Blob) => {
                    lut = Some(handle);
                }
                b"GAMMA_LUT_SIZE"
                    if matches!(info.value_type(), property::ValueType::UnsignedRange(_, _)) =>
                {
                    size = Some(handle);
                }
                _ => {}
            }
        }
        Ok(Self {
            crtc,
            lut: lut.ok_or_else(|| "CRTC has no GAMMA_LUT property".to_string())?,
            size: size.ok_or_else(|| "CRTC has no GAMMA_LUT_SIZE property".to_string())?,
            previous_blob: None,
        })
    }

    fn size(&self, device: &DrmDevice) -> Result<u32, String> {
        let properties = device
            .get_properties(self.crtc)
            .map_err(|err| format!("failed to query CRTC properties: {err}"))?;
        properties
            .into_iter()
            .find_map(|(handle, value)| (handle == self.size).then_some(value))
            .and_then(|value| u32::try_from(value).ok())
            .filter(|size| *size > 0)
            .ok_or_else(|| "CRTC reported an invalid GAMMA_LUT_SIZE".to_string())
    }

    fn set(&mut self, device: &DrmDevice, ramp: Option<&[u16]>) -> Result<(), String> {
        let new_blob = if let Some(ramp) = ramp {
            let size = self.size(device)? as usize;
            if ramp.len() != size * 3 {
                return Err("gamma ramp length changed during application".into());
            }
            let (red, rest) = ramp.split_at(size);
            let (green, blue) = rest.split_at(size);
            let mut lut = red
                .iter()
                .zip(green)
                .zip(blue)
                .map(|((&red, &green), &blue)| DrmColorLut {
                    red,
                    green,
                    blue,
                    reserved: 0,
                })
                .collect::<Vec<_>>();
            // `DrmColorLut` is a C-compatible four-u16 record with no
            // references or invalid bit patterns. DRM consumes these bytes
            // synchronously while creating its own property blob.
            let bytes = unsafe {
                std::slice::from_raw_parts_mut(
                    lut.as_mut_ptr().cast::<u8>(),
                    std::mem::size_of_val(lut.as_slice()),
                )
            };
            let blob = drm_ffi::mode::create_property_blob(device.as_fd(), bytes)
                .map_err(|err| format!("failed to create gamma LUT blob: {err}"))?;
            NonZeroU64::new(u64::from(blob.blob_id))
        } else {
            None
        };

        let raw = new_blob.map(NonZeroU64::get).unwrap_or(0);
        if let Err(err) =
            device.set_property(self.crtc, self.lut, property::Value::Blob(raw).into())
        {
            if raw != 0 {
                let _ = device.destroy_property_blob(raw);
            }
            return Err(format!("failed to set GAMMA_LUT: {err}"));
        }

        if let Some(previous) = std::mem::replace(&mut self.previous_blob, new_blob)
            && let Err(err) = device.destroy_property_blob(previous.get())
        {
            eventline::warn!("gamma: failed to destroy replaced LUT blob: {err}");
        }
        Ok(())
    }
}

impl Drop for AtomicGamma {
    fn drop(&mut self) {
        // The device owns blobs, but Drop has no device handle. Normal reset
        // paths destroy the last blob; process teardown lets DRM release it.
    }
}

fn set_legacy(device: &DrmDevice, crtc: crtc::Handle, ramp: Option<&[u16]>) -> Result<(), String> {
    let size = usize::try_from(
        device
            .get_crtc(crtc)
            .map_err(|err| format!("failed to query legacy gamma size: {err}"))?
            .gamma_length(),
    )
    .map_err(|_| "legacy gamma size does not fit in memory".to_string())?;
    if size == 0 {
        return Err("output does not support gamma ramps".into());
    }

    let linear;
    let ramp = if let Some(ramp) = ramp {
        if ramp.len() != size * 3 {
            return Err("legacy gamma ramp has the wrong length".into());
        }
        ramp
    } else {
        linear = linear_ramp(size);
        &linear
    };
    let (red, rest) = ramp.split_at(size);
    let (green, blue) = rest.split_at(size);
    device
        .set_gamma(crtc, red, green, blue)
        .map_err(|err| format!("failed to set legacy gamma ramp: {err}"))
}

fn linear_ramp(size: usize) -> Vec<u16> {
    let mut ramp = vec![0; size * 3];
    let denominator = size.saturating_sub(1).max(1) as u64;
    for index in 0..size {
        let value = if size == 1 {
            u16::MAX
        } else {
            (u64::from(u16::MAX) * index as u64 / denominator) as u16
        };
        ramp[index] = value;
        ramp[size + index] = value;
        ramp[size * 2 + index] = value;
    }
    ramp
}

#[cfg(test)]
mod tests {
    use super::linear_ramp;

    #[test]
    fn linear_reset_populates_all_three_channels() {
        assert_eq!(
            linear_ramp(3),
            vec![0, 32767, 65535, 0, 32767, 65535, 0, 32767, 65535]
        );
    }

    #[test]
    fn one_entry_ramp_does_not_divide_by_zero() {
        assert_eq!(linear_ramp(1), vec![65535, 65535, 65535]);
    }
}
