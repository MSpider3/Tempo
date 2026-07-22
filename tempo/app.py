"""QApplication factory for Tempo."""

from PySide6.QtWidgets import QApplication

from tempo.ui.theme import apply_theme


def create_app(argv: list[str]) -> QApplication:
    """Create and configure the Tempo QApplication.

    Args:
        argv: Command-line arguments (pass sys.argv).

    Returns:
        A fully configured QApplication instance.
    """
    app = QApplication(argv)
    app.setApplicationName("Tempo")
    app.setApplicationVersion("0.1.0")
    app.setOrganizationName("Tempo")

    # Fusion must be set BEFORE the stylesheet so dark theming works correctly
    # on all platforms (Linux, Windows, macOS).
    app.setStyle("Fusion")

    apply_theme(app)

    return app
