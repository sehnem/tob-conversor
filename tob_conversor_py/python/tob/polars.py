"""Polars convenience entry points."""

from __future__ import annotations

from pathlib import Path

import polars as pl

from tob import TobLazyFrame, read_tob, scan_tob


def read_tob_polars(
    path: str | Path,
    *,
    include_record: bool = False,
    utc: bool = True,
) -> pl.DataFrame:
    """Read a TOB file into a polars DataFrame."""
    return read_tob(path, engine="polars", include_record=include_record, utc=utc)


def scan_tob_polars(
    path: str | Path,
    *,
    include_record: bool = False,
) -> TobLazyFrame:
    """Return a lazy polars frame backed by a temp Arrow IPC file."""
    return scan_tob(path, include_record=include_record)
