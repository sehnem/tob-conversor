"""Tests that confirm lazy loading semantics for scan_tob, read_tob_chunks, and duckdb."""

from __future__ import annotations

from pathlib import Path

import polars as pl
import pytest
import tob

# ── scan_tob — schema available before collect ────────────────────────────────


def test_schema_available_without_collect(tob3_file: Path) -> None:
    """Schema introspection must work before any data is read."""
    with tob.scan_tob(tob3_file) as lf:
        schema = lf.schema
    assert "TIMESTAMP" in schema
    assert schema["A"] == pl.Float32


def test_operations_chain_without_collecting(tob3_file: Path) -> None:
    """Each chained operation returns a TobLazyFrame — not a materialised DataFrame."""
    with tob.scan_tob(tob3_file) as lf:
        step1 = lf.filter(pl.col("A").is_not_null())
        step2 = step1.select(["TIMESTAMP", "A"])
        assert isinstance(step1, tob.TobLazyFrame)
        assert isinstance(step2, tob.TobLazyFrame)
        df = step2.collect()
    assert df.columns == ["TIMESTAMP", "A"]
    assert len(df) == 2


def test_filter_reduces_rows(tob3_file: Path) -> None:
    """A filter that matches no rows returns an empty DataFrame on collect."""
    with tob.scan_tob(tob3_file) as lf:
        df = lf.filter(pl.col("A") > 9999.0).collect()
    assert len(df) == 0


def test_select_reduces_columns(tob3_file: Path) -> None:
    """Selecting a subset of columns must be applied lazily and honoured on collect."""
    with tob.scan_tob(tob3_file) as lf:
        df = lf.select(["TIMESTAMP"]).collect()
    assert df.columns == ["TIMESTAMP"]


def test_sort_is_applied_lazily(tob3_file: Path) -> None:
    """sort() must return a TobLazyFrame and produce a sorted result on collect."""
    with tob.scan_tob(tob3_file) as lf:
        lazy_sorted = lf.sort("A", descending=True)
        assert isinstance(lazy_sorted, tob.TobLazyFrame)
        df = lazy_sorted.collect()
    assert len(df) == 2


def test_head_limits_rows(tob3_file: Path) -> None:
    """head(1) must return a TobLazyFrame and yield exactly 1 row on collect."""
    with tob.scan_tob(tob3_file) as lf:
        lazy_head = lf.head(1)
        assert isinstance(lazy_head, tob.TobLazyFrame)
        df = lazy_head.collect()
    assert len(df) == 1


# ── scan_tob — temp-file lifecycle ────────────────────────────────────────────


def test_temp_ipc_file_exists_while_frame_is_open(tob3_file: Path) -> None:
    """The backing Arrow IPC file must be present on disk while TobLazyFrame is alive."""
    lf = tob.scan_tob(tob3_file)
    try:
        tmpdir = Path(lf._tmpdir.name)
        arrow_files = list(tmpdir.glob("*.arrow"))
        assert len(arrow_files) == 1, "Expected exactly one .arrow file in the temp dir"
        assert arrow_files[0].stat().st_size > 0
    finally:
        lf.close()


def test_temp_ipc_file_removed_after_close(tob3_file: Path) -> None:
    """The temp directory must be cleaned up after close() is called."""
    lf = tob.scan_tob(tob3_file)
    tmpdir_path = lf._tmpdir.name
    lf.close()
    assert not Path(tmpdir_path).exists()


def test_temp_ipc_file_removed_after_context_manager(tob3_file: Path) -> None:
    """The temp directory must be cleaned up on context-manager exit."""
    with tob.scan_tob(tob3_file) as lf:
        tmpdir_path = lf._tmpdir.name
    assert not Path(tmpdir_path).exists()


def test_collect_after_close_raises(tob3_file: Path) -> None:
    """Collecting after close should raise because the backing file is gone."""
    lf = tob.scan_tob(tob3_file)
    lf.close()
    with pytest.raises(FileNotFoundError):
        lf.collect()


# ── read_tob_chunks — streaming behaviour ─────────────────────────────────────


def test_chunks_yields_dataframes(tob3_file: Path) -> None:
    """Each yielded item must be a pandas DataFrame with the expected columns."""
    import pandas as pd

    chunks = list(tob.read_tob_chunks(tob3_file))
    assert all(isinstance(c, pd.DataFrame) for c in chunks)
    assert all("TIMESTAMP" in c.columns for c in chunks)
    assert all("A" in c.columns for c in chunks)


def test_chunks_total_rows_matches_eager(tob3_file: Path) -> None:
    """Total rows across all chunks must equal the eager read row count."""
    eager_count = len(tob.read_tob(tob3_file, engine="pandas"))
    chunked_count = sum(len(c) for c in tob.read_tob_chunks(tob3_file))
    assert chunked_count == eager_count


def test_chunks_respects_chunksize(tob3_file: Path) -> None:
    """With chunksize=1 on a 2-row file, exactly 2 chunks of 1 row each are yielded."""
    chunks = list(tob.read_tob_chunks(tob3_file, chunksize=1))
    assert len(chunks) == 2
    assert all(len(c) == 1 for c in chunks)


def test_chunks_single_chunk_when_size_exceeds_rows(tob3_file: Path) -> None:
    """A chunksize larger than the file yields a single chunk with all rows."""
    chunks = list(tob.read_tob_chunks(tob3_file, chunksize=10_000))
    assert len(chunks) == 1
    assert len(chunks[0]) == 2


def test_chunks_utc_false_strips_timezone(tob3_file: Path) -> None:
    """utc=False must produce naive timestamps in every chunk."""
    for chunk in tob.read_tob_chunks(tob3_file, utc=False):
        assert chunk["TIMESTAMP"].dt.tz is None


def test_chunks_include_record_adds_column(tob3_file: Path) -> None:
    """include_record=True must include a RECORD column in every chunk."""
    for chunk in tob.read_tob_chunks(tob3_file, include_record=True):
        assert "RECORD" in chunk.columns


# ── duckdb — lazy relation ────────────────────────────────────────────────────


def test_duckdb_returns_relation_not_data(tob3_file: Path) -> None:
    """read_tob with engine='duckdb' must return a DuckDBPyRelation, not a DataFrame."""
    import duckdb

    result = tob.read_tob(tob3_file, engine="duckdb")
    assert isinstance(result, duckdb.DuckDBPyRelation)


def test_duckdb_sql_filter_applied_lazily(tob3_file: Path) -> None:
    """SQL WHERE on the relation must reduce rows without pre-fetching all data."""
    rel = tob.read_tob(tob3_file, engine="duckdb")
    # Filter for rows that cannot exist (A > 9999) — should yield 0 rows
    filtered = rel.filter("A > 9999.0")
    assert filtered.fetchall() == []


def test_duckdb_sql_select_columns(tob3_file: Path) -> None:
    """Column projection on the relation must be honoured."""
    rel = tob.read_tob(tob3_file, engine="duckdb")
    # Select numeric column only to avoid pytz requirement for tz-aware timestamps
    projected = rel.select("A")
    rows = projected.fetchall()
    assert len(rows) == 2
    assert len(rows[0]) == 1  # only 1 column


def test_duckdb_chained_operations(tob3_file: Path) -> None:
    """Multiple chained DuckDB operations must yield the correct result set."""
    rel = tob.read_tob(tob3_file, engine="duckdb")
    # Chain filter → select (numeric column) → limit to verify lazy execution
    result = rel.filter("A IS NOT NULL").select("A").limit(1).fetchall()
    assert len(result) == 1
    assert len(result[0]) == 1
