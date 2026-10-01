use smithay::backend::allocator::Format;
use smithay::backend::allocator::format::FormatSet;
use smithay::wayland::dmabuf::DmabufFeedback;

/// Renderer facts needed to advertise one linux-dmabuf global.
///
/// Device discovery belongs to the concrete backend; protocol version and
/// fallback policy are decided by the shared Wayland layer.
#[derive(Clone, Debug)]
pub struct DmabufCapabilities {
    main_device: Option<libc::dev_t>,
    formats: FormatSet,
}

impl DmabufCapabilities {
    pub fn new(main_device: Option<libc::dev_t>, formats: FormatSet) -> Self {
        Self {
            main_device,
            formats,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.formats.iter().next().is_none()
    }

    pub fn main_device(&self) -> Option<libc::dev_t> {
        self.main_device
    }

    pub fn formats(&self) -> &FormatSet {
        &self.formats
    }
}

#[derive(Clone, Debug)]
pub struct SurfaceDmabufFeedback {
    pub render: DmabufFeedback,
    pub scanout: DmabufFeedback,
}

/// Formats that are both importable by the renderer and accepted by a KMS
/// plane. The complete `(fourcc, modifier)` pair must match on both sides.
pub fn scanout_formats(renderer: &FormatSet, plane: &FormatSet) -> Vec<Format> {
    plane.intersection(renderer).copied().collect()
}

#[cfg(test)]
mod tests {
    use smithay::backend::allocator::{Fourcc, Modifier};

    use super::*;

    fn format(code: Fourcc, modifier: Modifier) -> Format {
        Format { code, modifier }
    }

    #[test]
    fn scanout_requires_matching_code_and_modifier() {
        let linear_xrgb = format(Fourcc::Xrgb8888, Modifier::Linear);
        let tiled_xrgb = format(Fourcc::Xrgb8888, Modifier::Invalid);
        let linear_argb = format(Fourcc::Argb8888, Modifier::Linear);
        let renderer: FormatSet = [linear_xrgb, tiled_xrgb].into_iter().collect();
        let plane: FormatSet = [linear_xrgb, linear_argb].into_iter().collect();

        assert_eq!(scanout_formats(&renderer, &plane), vec![linear_xrgb]);
    }

    #[test]
    fn empty_capabilities_are_detectable_without_device_information() {
        assert!(DmabufCapabilities::new(None, FormatSet::default()).is_empty());
    }
}
