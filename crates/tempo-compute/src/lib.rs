//! tempo-compute: IPC client for external Python compute server.

pub mod client;
pub mod error;

pub use client::{compute_script_path, default_socket_path, ComputeClient};
pub use error::ComputeError;
