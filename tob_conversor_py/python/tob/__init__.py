from __future__ import annotations

import tempfile
from collections.abc import Iterator
from pathlib import Path
from typing import TYPE_CHECKING, Literal, overload

if TYPE_CHECKING:
    import duckdb
    import pandas as pd
    import polars as pl

from tob._core import Header, to_parquet
from tob._core import read_header as _read_header
from tob._lazy import TobLazyFrame
from tob.duckdb import open_tob

__all__ = [
    "Header",
    "TobLazyFrame",
    "open_tob",
    "read_header",
    "read_tob",
    "read_tob_chunks",
    "scan_tob",
    "to_parquet",
]


def read_header(path: str | Path) -> Header:
    """Parse the 6-line ASCII TOB header without reading binary data."""
    return _read_header(str(Path(path)))


@overload
def read_tob(
    path: str | Path,
    *,
    engine: Literal["pandas"] = ...,
    include_record: bool = ...,
    utc: bool = ...,
) -> pd.DataFrame: ...
@overload
def read_tob(
    path: str | Path, *, engine: Literal["polars"], include_record: bool = ..., utc: bool = ...
) -> pl.DataFrame: ...
@overload
def read_tob(
    path: str | Path, *, engine: Literal["duckdb"], include_record: bool = ..., utc: bool = ...
) -> duckdb.DuckDBPyRelation: ...
def read_tob(
    path: str | Path,
    *,
    engine: Literal["pandas", "polars", "duckdb"] = "pandas",
    include_record: bool = False,
    utc: bool = True,
) -> pd.DataFrame | pl.DataFrame | duckdb.DuckDBPyRelation:
    """Read a TOB file eagerly into a DataFrame.

    Parameters
    ----------
    path:
        Path to the TOB binary file.
    engine:
        Output format — ``"pandas"``, ``"polars"``, or ``"duckdb"``.
    include_record:
        Include the logger RECORD number as a column.
    utc:
        When ``False``, strip the UTC timezone from the TIMESTAMP column so it
        becomes a naive datetime.  The values are not shifted — they remain UTC.
    """
    from tob._arrow import ipc_bytes_to_table
    from tob._core import to_arrow_ipc

    ipc_bytes = to_arrow_ipc(str(Path(path)), include_record)
    if not ipc_bytes:
        raise ValueError(f"No data written from {path!r} — file may be empty or invalid")

    table = ipc_bytes_to_table(ipc_bytes)

    if engine == "pandas":
        return _read_pandas(table, utc=utc)
    elif engine == "polars":
        return _read_polars(table, utc=utc)
    elif engine == "duckdb":
        return _read_duckdb(table)
    else:
        raise ValueError(f"Unknown engine {engine!r}. Choose 'pandas', 'polars', or 'duckdb'")


def scan_tob(
    path: str | Path,
    *,
    include_record: bool = False,
) -> TobLazyFrame:
    """Return a lazy Polars frame backed by a temporary Arrow IPC file.

    The IPC file is kept on disk until the ``TobLazyFrame`` is garbage-
    collected or explicitly closed.  Use as a context manager for deterministic
    cleanup::

        with tob.scan_tob("data.dat") as lf:
            result = lf.filter(pl.col("AirTC") > 20).collect()
    """
    from tob._core import to_arrow_ipc_file_py

    path = Path(path)
    tmpdir = tempfile.TemporaryDirectory()
    try:
        out_path = Path(tmpdir.name) / "data.arrow"
        rows = to_arrow_ipc_file_py(str(path), str(out_path), include_record)
        if rows == 0:
            tmpdir.cleanup()
            raise ValueError(f"No data written from {path!r} — file may be empty or invalid")
        import polars as pl

        frame = pl.scan_ipc(out_path)
        return TobLazyFrame(frame, tmpdir)
    except Exception:
        tmpdir.cleanup()
        raise


def read_tob_chunks(
    path: str | Path,
    *,
    chunksize: int = 65536,
    include_record: bool = False,
    utc: bool = True,
) -> Iterator[pd.DataFrame]:
    """Iterate over a TOB file in chunks, yielding one pandas DataFrame per batch.

    Suitable for files larger than available RAM: only one batch is held in
    memory at a time.

    Parameters
    ----------
    path:
        Path to the TOB binary file.
    chunksize:
        Approximate number of rows per yielded DataFrame.
    include_record:
        Include the logger RECORD number as a column.
    utc:
        When ``False``, strip the UTC timezone from the TIMESTAMP column.
    """
    from tob._arrow import ipc_stream_chunk_to_record_batch
    from tob._core import TobReader

    for chunk_bytes in TobReader(str(Path(path)), include_record, chunksize):
        rb = ipc_stream_chunk_to_record_batch(chunk_bytes)
        df = rb.to_pandas()
        if not utc:
            df["TIMESTAMP"] = df["TIMESTAMP"].dt.tz_convert(None)
        yield df


# ── Engine implementations ────────────────────────────────────────────────────


def _read_pandas(table, *, utc: bool) -> pd.DataFrame:
    df = table.to_pandas()
    if not utc:
        df["TIMESTAMP"] = df["TIMESTAMP"].dt.tz_convert(None)
    return df


def _read_polars(table, *, utc: bool) -> pl.DataFrame:
    from typing import cast

    import polars as pl

    df = cast(pl.DataFrame, pl.from_arrow(table))
    if not utc:
        df = df.with_columns(pl.col("TIMESTAMP").dt.replace_time_zone(None))
    return df


def _read_duckdb(table) -> duckdb.DuckDBPyRelation:
    import duckdb

    conn = duckdb.connect()
    conn.register("tob_data", table)
    return conn.sql("SELECT * FROM tob_data")
