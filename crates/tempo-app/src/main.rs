//! Tempo — a light video editor for Linux.

use tempo_ui::TempoApp;

fn main() -> glib::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    // `tempo --smoke-test` starts the interface and quits once the window is up.
    // `tempo project.tempo` opens that project directly.
    for arg in std::env::args().skip(1) {
        if arg == "--smoke-test" {
            std::env::set_var("TEMPO_SMOKE_TEST", "1");
        } else if !arg.starts_with('-') {
            std::env::set_var("TEMPO_OPEN", arg);
        }
    }

    TempoApp::new().run()
}
