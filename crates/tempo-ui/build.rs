//! Packs the icons in assets/icons into a GResource bundle that is built into the program.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    let assets = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let manifest = assets.join("tempo.gresource.xml");
    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rerun-if-changed={}", assets.join("icons").display());

    let out_dir = std::env::var("OUT_DIR").expect("cargo sets OUT_DIR for build scripts");
    let target = PathBuf::from(out_dir).join("tempo.gresource");
    let status = Command::new("glib-compile-resources")
        .arg(format!("--sourcedir={}", assets.display()))
        .arg(format!("--target={}", target.display()))
        .arg(&manifest)
        .status();
    match status {
        Ok(s) if s.success() => {}
        Ok(s) => panic!("glib-compile-resources failed: {s}"),
        Err(e) => panic!("glib-compile-resources is needed to build Tempo (glib2-devel / libglib2.0-dev-bin): {e}"),
    }
}
