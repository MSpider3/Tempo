use thiserror::Error;

#[derive(Error, Debug)]
pub enum RenderError {
    #[error("Failed to request wgpu adapter")]
    AdapterRequestFailed,

    #[error("Failed to request wgpu device: {0}")]
    DeviceRequestFailed(#[from] wgpu::RequestDeviceError),

    #[error("Buffer mapping failed")]
    BufferMapFailed,

    #[error("Surface error: {0}")]
    SurfaceError(String),

    #[error("Title render error: {0}")]
    TitleError(String),
}

pub type Result<T> = std::result::Result<T, RenderError>;
