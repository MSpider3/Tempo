use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=../../assets/tempo.gresource.xml");
    println!("cargo:rerun-if-changed=../../assets/icons");
    println!("cargo:rerun-if-changed=../../assets/tempo.gresource");

    let out_dir = env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("tempo.gresource");

    let status = Command::new("glib-compile-resources")
        .args([
            "--sourcedir=../../assets",
            &format!("--target={}", dest_path.display()),
            "../../assets/tempo.gresource.xml",
        ])
        .status();

    let success = matches!(status, Ok(s) if s.success());
    if !success {
        // Fall back to pre-compiled gresource in assets/
        let fallback = Path::new("../../assets/tempo.gresource");
        if fallback.exists() {
            fs::copy(fallback, &dest_path).expect("failed to copy fallback tempo.gresource");
        } else {
            panic!("glib-compile-resources failed and assets/tempo.gresource not found");
        }
    }
}
