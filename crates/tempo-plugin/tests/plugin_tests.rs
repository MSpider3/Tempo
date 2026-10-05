use std::path::Path;
use std::sync::{Arc, Mutex};
use tempo_plugin::{LuaEngine, PluginError, PluginManager};
use tempo_timeline::command::CommandLog;
use tempo_timeline::{Timeline, Track, TrackKind};

static INIT_PLUGINS: std::sync::Once = std::sync::Once::new();

fn ensure_test_plugins() {
    INIT_PLUGINS.call_once(|| {
        let target_dirs = [
            Path::new("tests/plugins"),
            Path::new("crates/tempo-plugin/tests/plugins"),
        ];

        // 1. crashing.wasm: triggers unreachable trap in plugin_init
        let crashing_wat = r#"
            (module
                (func $init (result i32)
                    (unreachable)
                    (i32.const 0)
                )
                (export "plugin_init" (func $init))
                (memory (export "memory") 1)
            )
        "#;
        let crashing_wasm = wat::parse_str(crashing_wat).expect("valid crashing wat");

        // 2. memory_limit.wasm: tries to allocate >256 MB (5000 pages * 64KB = 320 MB > 256MB limit)
        let memory_wat = r#"
            (module
                (memory (export "memory") 1)
                (func $init (result i32)
                    ;; Try to grow memory by 5000 pages (320 MB)
                    (memory.grow (i32.const 5000))
                    ;; Returns -1 if memory grow failed (which it should under 256MB limit)
                )
                (export "plugin_init" (func $init))
            )
        "#;
        let memory_wasm = wat::parse_str(memory_wat).expect("valid memory wat");

        // 3. test_effect_plugin.wasm: registers glow transition shader
        let effect_wat = r#"
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
        let effect_wasm = wat::parse_str(effect_wat).expect("valid effect wat");

        for dir in &target_dirs {
            let _ = std::fs::create_dir_all(dir);
            let _ = std::fs::write(dir.join("crashing.wasm"), &crashing_wasm);
            let _ = std::fs::write(dir.join("memory_limit.wasm"), &memory_wasm);
            let _ = std::fs::write(dir.join("test_effect_plugin.wasm"), &effect_wasm);
        }
    });
}

fn resolve_plugin_path(filename: &str) -> std::path::PathBuf {
    let p1 = Path::new("tests/plugins").join(filename);
    if p1.exists() {
        return p1;
    }
    let p2 = Path::new("crates/tempo-plugin/tests/plugins").join(filename);
    if p2.exists() {
        return p2;
    }
    p1
}

#[test]
fn plugin_crash_isolation() {
    ensure_test_plugins();

    let mut manager = PluginManager::new().expect("PluginManager initialization");
    let crashing_path = resolve_plugin_path("crashing.wasm");
    assert!(crashing_path.exists());

    // Load the crashing plugin
    manager
        .load_plugin_file("crashing-plugin", &crashing_path)
        .expect("Loading crashing module should succeed");

    // Enabling calls plugin_init, which executes (unreachable)
    let res = manager.enable("crashing-plugin");

    // The trap must be caught gracefully and return PluginError::Trapped
    match res {
        Err(PluginError::Trapped(msg)) => {
            println!("Caught expected trap: {}", msg);
            assert!(!msg.is_empty());
        }
        other => panic!("Expected trapped error, got {:?}", other),
    }

    // Plugin must not be enabled, and host didn't crash
    assert!(!manager.is_enabled("crashing-plugin"));
}

#[test]
fn plugin_memory_limit() {
    ensure_test_plugins();

    let mut manager = PluginManager::new().expect("PluginManager initialization");
    let mem_path = resolve_plugin_path("memory_limit.wasm");
    assert!(mem_path.exists());

    manager
        .load_plugin_file("mem-plugin", &mem_path)
        .expect("Loading memory module should succeed");

    // plugin_init calls memory.grow 5000 (320 MB > 256 MB)
    // Under wasmtime's 256MB StoreLimit, memory.grow returns -1
    // plugin_init returns -1, which manager translates into InitFailed(-1)
    let res = manager.enable("mem-plugin");
    match res {
        Err(PluginError::InitFailed(code)) => {
            println!("Memory grow allocation rejected with code: {}", code);
            assert_eq!(code, -1);
        }
        Err(PluginError::Trapped(msg)) => {
            println!("Memory limit triggered trap: {}", msg);
        }
        other => panic!("Expected InitFailed or Trapped, got {:?}", other),
    }

    assert!(!manager.is_enabled("mem-plugin"));
}

#[test]
fn code_mode_lua() {
    let mut timeline = Timeline::new();
    let audio_track = Track::new(TrackKind::Audio, 0, "A1", 0);
    timeline.tracks.push(audio_track);

    let tl_arc = Arc::new(Mutex::new(timeline));
    let cmd_log = Arc::new(Mutex::new(CommandLog::new()));

    let lua_engine = LuaEngine::new(tl_arc.clone(), cmd_log.clone())
        .expect("LuaEngine initialization");

    let script = r#"
        timeline.insert_clip('A1', 'tests/media/sample_audio_only.mp3', 0, 5000000)
    "#;

    lua_engine.execute_script(script).expect("Execution of Lua script");

    // Verify clip appears on timeline
    {
        let guard = tl_arc.lock().unwrap();
        let track = guard.tracks.iter().find(|t| t.name == "A1").expect("Track A1 exists");
        assert_eq!(track.clips.len(), 1);
        assert_eq!(track.clips[0].name, "sample_audio_only.mp3");
        assert_eq!(track.clips[0].source_in, 0);
        assert_eq!(track.clips[0].source_out, 5000000);
    }

    // Verify operation is undo-able
    {
        let mut guard = tl_arc.lock().unwrap();
        let mut cmd_guard = cmd_log.lock().unwrap();
        assert!(cmd_guard.can_undo());
        cmd_guard.undo(&mut guard).expect("Undo successful");

        let track = guard.tracks.iter().find(|t| t.name == "A1").unwrap();
        assert_eq!(track.clips.len(), 0, "Clip removed on undo");

        assert!(cmd_guard.can_redo());
        cmd_guard.redo(&mut guard).expect("Redo successful");
        let track = guard.tracks.iter().find(|t| t.name == "A1").unwrap();
        assert_eq!(track.clips.len(), 1, "Clip restored on redo");
    }
}
