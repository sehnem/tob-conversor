"""Pandas convenience entry points."""

from __future__ import annotations

from pathlib import Path

import pandas as pd

from tob import read_tob


def read_tob_pandas(
    path: str | Path,
    *,
    include_record: bool = False,
    utc: bool = True,
) -> pd.DataFrame:
    """Read a TOB file into a pandas DataFrame."""
    return read_tob(path, engine="pandas", include_record=include_record, utc=utc)
