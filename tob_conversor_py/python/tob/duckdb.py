"""DuckDB convenience entry points."""

from __future__ import annotations

import tempfile
from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    import duckdb


def read_tob_duckdb(
    path: str | Path,
    *,
    include_record: bool = False,
) -> duckdb.DuckDBPyRelation:
    """Read a TOB file into a DuckDB in-memory relation."""
    from tob import read_tob

    return read_tob(path, engine="duckdb", include_record=include_record)


@contextmanager
def open_tob(
    path: str | Path,
    *,
    conn: duckdb.DuckDBPyConnection | None = None,
    table: str = "tob",
    columns: list[str] | None = None,
    include_record: bool = False,
) -> Iterator[duckdb.DuckDBPyConnection]:
    """Context manager that opens a TOB file as a DuckDB view.

    Converts the TOB binary to Parquet once (via Rust), then registers a view
    over only the requested columns.  The Parquet file and the view are cleaned
    up automatically on exit.

    Parameters
    ----------
    path:
        Path to the TOB binary file.
    conn:
        Existing DuckDB connection to use.  A new in-memory connection is
        created when not provided.
    table:
        Name of the view created inside DuckDB.  Defaults to ``"tob"``.
    columns:
        Column names to expose in the view.  ``None`` exposes all columns.
    include_record:
        Include the logger RECORD number as a column.

    Example
    -------
    ::

        with tob.open_tob("data.dat", columns=["TIMESTAMP", "w", "diag_CSAT"]) as conn:
            df = conn.sql(
                "SELECT avg(w) FILTER (WHERE diag_CSAT = 0) FROM tob"
            ).df()
    """
    import duckdb as _duckdb

    from tob._core import to_parquet

    path = Path(path)
    if conn is None:
        conn = _duckdb.connect()
        own_conn = True
    else:
        own_conn = False

    with tempfile.TemporaryDirectory() as tmp:
        to_parquet(str(path), tmp, include_record)
        parquet = Path(tmp) / (path.stem + ".parquet")

        col_sel = ", ".join(columns) if columns else "*"
        conn.execute(
            f"CREATE OR REPLACE VIEW {table} AS SELECT {col_sel} FROM read_parquet('{parquet}')"
        )
        try:
            yield conn
        finally:
            conn.execute(f"DROP VIEW IF EXISTS {table}")
            if own_conn:
                conn.close()
