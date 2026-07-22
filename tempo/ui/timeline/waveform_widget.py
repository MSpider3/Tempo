"""Waveform QGraphicsItem renderer for audio clips on the timeline."""

from __future__ import annotations

from PySide6.QtCore import QPointF, QRectF
from PySide6.QtGui import QColor, QPainter, QPen
from PySide6.QtWidgets import QGraphicsItem, QStyleOptionGraphicsItem, QWidget

from tempo.ui.theme import COLORS


def subsample_peaks(
    peaks: list[tuple[float, float]], target_count: int
) -> list[tuple[float, float]]:
    """Reduce peaks list to target_count samples by merging buckets.

    Each output sample is the min of mins and max of maxes in its bucket.
    """
    if not peaks or target_count <= 0 or len(peaks) <= target_count:
        return peaks
    bucket_size = len(peaks) / target_count
    result: list[tuple[float, float]] = []
    for i in range(target_count):
        start = int(i * bucket_size)
        end = int((i + 1) * bucket_size)
        if end <= start:
            end = start + 1
        bucket = peaks[start:end]
        if bucket:
            result.append((min(p[0] for p in bucket), max(p[1] for p in bucket)))
    return result


class WaveformItem(QGraphicsItem):
    """Draws a waveform inside an audio ClipItem.

    Owned by ClipItem — positioned relative to the clip's local coordinates.
    """

    def __init__(
        self,
        peaks: list[tuple[float, float]],
        width: float,
        height: float,
    ) -> None:
        super().__init__()
        self._raw_peaks = peaks
        self._width = width
        self._height = height
        self.setZValue(0)  # below clip label text (zValue=1)
        self._cached_peaks: list[tuple[float, float]] = []
        self._recalculate_peaks()

    def _recalculate_peaks(self) -> None:
        target = max(1, int(self._width))
        self._cached_peaks = subsample_peaks(self._raw_peaks, target)

    def boundingRect(self) -> QRectF:  # noqa: N802
        return QRectF(0.0, 0.0, float(self._width), float(self._height))

    def paint(
        self,
        painter: QPainter,
        option: QStyleOptionGraphicsItem | None = None,
        widget: QWidget | None = None,
    ) -> None:
        if not self._cached_peaks:
            return
        painter.setRenderHint(QPainter.RenderHint.Antialiasing, False)
        waveform_color = QColor(COLORS["waveform"])  # #5AE8A0
        waveform_color.setAlpha(180)
        painter.setPen(QPen(waveform_color, 1.0))

        n = len(self._cached_peaks)
        cy = self._height / 2.0  # vertical center

        for i, (lo, hi) in enumerate(self._cached_peaks):
            x = (i / max(n - 1, 1)) * self._width
            lo_clamped = max(-1.0, min(0.0, lo))
            hi_clamped = max(0.0, min(1.0, hi))
            y_top = cy - hi_clamped * cy
            y_bot = cy - lo_clamped * cy
            painter.drawLine(QPointF(x, y_top), QPointF(x, y_bot))

    def set_size(self, width: float, height: float) -> None:
        self.prepareGeometryChange()
        self._width = max(1.0, width)
        self._height = max(1.0, height)
        self._recalculate_peaks()
        self.update()

    def set_peaks(self, peaks: list[tuple[float, float]]) -> None:
        self._raw_peaks = peaks
        self._recalculate_peaks()
        self.update()
