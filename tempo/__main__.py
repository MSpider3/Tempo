"""Application entry point for Tempo."""

import sys

from tempo.app import create_app
from tempo.ui.main_window import MainWindow


def main() -> None:
    """Launch the Tempo application."""
    app = create_app(sys.argv)
    window = MainWindow()
    window.show()
    sys.exit(app.exec())


if __name__ == "__main__":
    main()
