use tempo_compute::{compute_script_path, ComputeClient};

#[tokio::test]
async fn server_ping() {
    let test_socket = std::env::temp_dir().join(format!("tempo-compute-test-{}.sock", std::process::id()));
    let script_path = compute_script_path();

    assert!(script_path.exists(), "compute.py must exist at {:?}", script_path);

    // Start compute server using the custom test socket
    let client = ComputeClient::ensure_started_with_path(&test_socket, &script_path)
        .await
        .expect("Failed to start or connect to compute server");

    // Send ping request
    let pong = client.ping().await.expect("Ping request failed");
    assert!(pong, "Ping should return true / status ok");

    // Clean up test socket
    if test_socket.exists() {
        let _ = std::fs::remove_file(test_socket);
    }
}
