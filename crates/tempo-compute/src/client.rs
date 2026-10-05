use crate::error::ComputeError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::process::Command;
use tracing::{debug, info};

#[derive(Debug, Serialize)]
struct JsonRpcRequest<'a> {
    jsonrpc: &'static str,
    id: u64,
    method: &'a str,
    params: Value,
}

#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    #[allow(dead_code)]
    #[serde(default)]
    id: Option<u64>,
    result: Option<Value>,
    error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

pub struct ComputeClient {
    socket_path: PathBuf,
    req_id: AtomicU64,
}

pub fn default_socket_path() -> PathBuf {
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        PathBuf::from(runtime_dir).join("tempo-compute.sock")
    } else {
        std::env::temp_dir().join("tempo-compute.sock")
    }
}

pub fn compute_script_path() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // Crate is at crates/tempo-compute, root is manifest_dir/../..
    let root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or(&manifest_dir);
    root.join("server").join("compute.py")
}

impl ComputeClient {
    pub fn new(socket_path: PathBuf) -> Self {
        Self {
            socket_path,
            req_id: AtomicU64::new(1),
        }
    }

    pub async fn connect(socket_path: PathBuf) -> Result<Self, ComputeError> {
        // Quick connection test
        let _ = UnixStream::connect(&socket_path).await?;
        Ok(Self::new(socket_path))
    }

    pub async fn ensure_started_with_path(
        socket_path: &Path,
        script_path: &Path,
    ) -> Result<Self, ComputeError> {
        if socket_path.exists() {
            if let Ok(client) = Self::connect(socket_path.to_path_buf()).await {
                if client.ping().await.unwrap_or(false) {
                    info!("Connected to existing compute server at {:?}", socket_path);
                    return Ok(client);
                }
            }
            // Stale socket
            let _ = std::fs::remove_file(socket_path);
        }

        info!("Spawning compute server from {:?}", script_path);
        let mut cmd = Command::new("python3");
        cmd.arg(script_path)
            .arg("--socket")
            .arg(socket_path)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());

        let _child = cmd.spawn()?;

        // Wait up to 10 seconds for the socket to appear
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if socket_path.exists() {
                if let Ok(client) = Self::connect(socket_path.to_path_buf()).await {
                    if client.ping().await.unwrap_or(false) {
                        info!("Compute server ready at {:?}", socket_path);
                        return Ok(client);
                    }
                }
            }
        }

        Err(ComputeError::ServerStartTimeout)
    }

    pub async fn ensure_started() -> Result<Self, ComputeError> {
        let sock = default_socket_path();
        let script = compute_script_path();
        Self::ensure_started_with_path(&sock, &script).await
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value, ComputeError> {
        let id = self.req_id.fetch_add(1, Ordering::SeqCst);
        let request = JsonRpcRequest {
            jsonrpc: "2.0",
            id,
            method,
            params,
        };

        let mut payload = serde_json::to_vec(&request)?;
        payload.push(b'\n');

        let mut stream = UnixStream::connect(&self.socket_path).await?;
        stream.write_all(&payload).await?;
        stream.flush().await?;

        let mut reader = BufReader::new(stream);
        let mut response_line = String::new();
        let n = reader.read_line(&mut response_line).await?;
        if n == 0 {
            return Err(ComputeError::Protocol("Server closed connection".to_string()));
        }

        let resp: JsonRpcResponse = serde_json::from_str(&response_line)?;
        if let Some(err) = resp.error {
            return Err(ComputeError::Rpc {
                code: err.code,
                message: err.message,
            });
        }

        resp.result
            .ok_or_else(|| ComputeError::Protocol("Empty result in JSON-RPC response".to_string()))
    }

    pub async fn ping(&self) -> Result<bool, ComputeError> {
        match self.call("ping", Value::Object(Default::default())).await {
            Ok(val) => {
                let status = val.get("status").and_then(|s| s.as_str()).unwrap_or("");
                Ok(status == "ok")
            }
            Err(e) => {
                debug!("Ping failed: {}", e);
                Ok(false)
            }
        }
    }
}
