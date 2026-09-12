from __future__ import annotations

import tempfile
from typing import TYPE_CHECKING, Any

if TYPE_CHECKING:
    import polars as pl


class TobLazyFrame:
    """Polars LazyFrame backed by a temporary Arrow IPC file on disk.

    The temp directory is kept alive until this object is garbage-collected or
    ``close()`` / the context manager is used.  Delegate lazy operations by
    wrapping the inner frame; ``collect()`` materialises data into memory.
    """

    def __init__(self, frame: pl.LazyFrame, tmpdir: tempfile.TemporaryDirectory) -> None:
        self._frame = frame
        self._tmpdir = tmpdir

    # ── Core ──────────────────────────────────────────────────────────────────

    def collect(self) -> pl.DataFrame:
        from typing import cast

        import polars as pl

        return cast(pl.DataFrame, self._frame.collect())

    @property
    def schema(self) -> pl.Schema:
        return self._frame.collect_schema()

    # ── Lazy delegation ───────────────────────────────────────────────────────
    # Rewrap the result so the tmpdir reference travels with every derived frame.

    def filter(self, *args: Any, **kwargs: Any) -> TobLazyFrame:
        return TobLazyFrame(self._frame.filter(*args, **kwargs), self._tmpdir)

    def select(self, *args: Any, **kwargs: Any) -> TobLazyFrame:
        return TobLazyFrame(self._frame.select(*args, **kwargs), self._tmpdir)

    def with_columns(self, *args: Any, **kwargs: Any) -> TobLazyFrame:
        return TobLazyFrame(self._frame.with_columns(*args, **kwargs), self._tmpdir)

    def sort(self, *args: Any, **kwargs: Any) -> TobLazyFrame:
        return TobLazyFrame(self._frame.sort(*args, **kwargs), self._tmpdir)

    def limit(self, n: int) -> TobLazyFrame:
        return TobLazyFrame(self._frame.limit(n), self._tmpdir)

    def head(self, n: int = 5) -> TobLazyFrame:
        return TobLazyFrame(self._frame.head(n), self._tmpdir)

    # ── Lifecycle ─────────────────────────────────────────────────────────────

    def close(self) -> None:
        """Explicitly release the temp Arrow IPC file before GC."""
        self._tmpdir.cleanup()

    def __enter__(self) -> TobLazyFrame:
        return self

    def __exit__(self, *_: object) -> None:
        self.close()

    def __repr__(self) -> str:
        return f"TobLazyFrame(schema={self.schema})"
