use std::path::Path;

pub struct VaapiHwContext {
    pub device_path: String,
    pub is_available: bool,
}

impl VaapiHwContext {
    pub fn new() -> Option<Self> {
        let default_device = "/dev/dri/renderD128";
        if Path::new(default_device).exists() {
            tracing::info!("VAAPI device node detected at {}", default_device);
            Some(Self {
                device_path: default_device.to_string(),
                is_available: true,
            })
        } else {
            tracing::info!("No VAAPI render node found at {}; continuing with software decode", default_device);
            None
        }
    }
}
