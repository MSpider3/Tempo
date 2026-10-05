use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tempo_plugin::PluginManager;
use tempo_timeline::command::CommandLog;
use tempo_timeline::Timeline;

fn ensure_test_effect_plugin(path: &Path) {
    if path.exists() {
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let wat_str = r#"
        (module
            (import "tempo" "render_register_effect" (func $register_effect (param i32 i32 i32 i32) (result i32)))
            (import "tempo" "render_unregister_effect" (func $unregister_effect (param i32 i32)))
            (memory (export "memory") 1)
            (data (i32.const 100) "my-plugin.glow-transition")
            (data (i32.const 200) "@fragment\nfn fs_main() -> @location(0) vec4f { return vec4f(1.0, 0.5, 0.0, 1.0); }")

            (func $init (result i32)
                (call $register_effect (i32.const 100) (i32.const 25) (i32.const 200) (i32.const 75))
            )
            (func $shutdown
                (call $unregister_effect (i32.const 100) (i32.const 25))
            )
            (export "plugin_init" (func $init))
            (export "plugin_shutdown" (func $shutdown))
        )
    "#;
    let wasm_bytes = wat::parse_str(wat_str).expect("Valid WAT string");
    std::fs::write(path, wasm_bytes).expect("Write test_effect_plugin.wasm");
}

fn main() {
    tracing_subscriber::fmt::init();

    let wasm_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("tests/plugins/test_effect_plugin.wasm"));

    ensure_test_effect_plugin(&wasm_path);

    println!("Testing effect plugin at: {:?}", wasm_path);

    let mut manager = PluginManager::new().expect("PluginManager::new");
    let timeline = Arc::new(Mutex::new(Timeline::new()));
    let command_log = Arc::new(Mutex::new(CommandLog::new()));
    manager.set_timeline(timeline, command_log);

    let plugin_id = "test-effect-plugin";

    // 1. Load plugin
    manager
        .load_plugin_file(plugin_id, &wasm_path)
        .expect("load_plugin_file should succeed");

    println!("✓ Plugin binary loaded successfully");

    // 2. Enable plugin (executes plugin_init and calls render_register_effect)
    manager.enable(plugin_id).expect("enable should succeed");
    assert!(manager.is_enabled(plugin_id));

    // 3. Verify effect registered in tempo effect library
    let effects = manager.registered_effects();
    let effect_id = "my-plugin.glow-transition";
    {
        let guard = effects.lock().unwrap();
        assert!(
            guard.contains_key(effect_id),
            "Expected registered effect '{}', found {:?}",
            effect_id,
            guard.keys().collect::<Vec<_>>()
        );
        let wgsl = guard.get(effect_id).unwrap();
        assert!(wgsl.contains("fs_main"), "WGSL shader code must match");
        println!("✓ Effect '{}' registered with WGSL shader", effect_id);
    }

    // 4. Verify effect cleanly unregistered on unload / disable
    manager.disable(plugin_id).expect("disable should succeed");
    assert!(!manager.is_enabled(plugin_id));

    {
        let guard = effects.lock().unwrap();
        assert!(
            !guard.contains_key(effect_id),
            "Expected effect '{}' to be unregistered after disable",
            effect_id
        );
        println!("✓ Effect '{}' cleanly unregistered on shutdown", effect_id);
    }

    println!("=== Plugin Effect Test Passed ===");
}
