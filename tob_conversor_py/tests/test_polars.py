"""Tests for the polars engine (eager and lazy)."""

from __future__ import annotations

from pathlib import Path

import polars as pl
import pytest
import tob


def test_returns_dataframe(tob3_file: Path) -> None:
    df = tob.read_tob(tob3_file, engine="polars")
    assert isinstance(df, pl.DataFrame)


def test_row_count(tob3_file: Path) -> None:
    df = tob.read_tob(tob3_file, engine="polars")
    assert len(df) == 2


def test_column_names(tob3_file: Path) -> None:
    df = tob.read_tob(tob3_file, engine="polars")
    assert "TIMESTAMP" in df.columns
    assert "A" in df.columns


def test_float32_dtype(tob3_file: Path) -> None:
    df = tob.read_tob(tob3_file, engine="polars")
    assert df["A"].dtype == pl.Float32


def test_timestamp_dtype_utc(tob3_file: Path) -> None:
    df = tob.read_tob(tob3_file, engine="polars")
    assert df["TIMESTAMP"].dtype == pl.Datetime("ns", "UTC")


def test_timestamp_dtype_naive(tob3_file: Path) -> None:
    df = tob.read_tob(tob3_file, engine="polars", utc=False)
    assert df["TIMESTAMP"].dtype == pl.Datetime("ns", None)


def test_fp2_value(tob3_file: Path) -> None:
    df = tob.read_tob(tob3_file, engine="polars")
    assert df["A"][0] == pytest.approx(14.22, abs=1e-4)


def test_null_propagation(tob3_file_with_nan: Path) -> None:
    df = tob.read_tob(tob3_file_with_nan, engine="polars")
    assert df["A"][0] is not None
    assert df["B"][0] is None


# ── Lazy (scan_tob) ───────────────────────────────────────────────────────────


def test_scan_tob_returns_lazy_frame(tob3_file: Path) -> None:
    lf = tob.scan_tob(tob3_file)
    assert isinstance(lf, tob.TobLazyFrame)
    lf.close()


def test_scan_tob_collect(tob3_file: Path) -> None:
    with tob.scan_tob(tob3_file) as lf:
        df = lf.collect()
    assert len(df) == 2
    assert "A" in df.columns


def test_scan_tob_filter(tob3_file: Path) -> None:
    with tob.scan_tob(tob3_file) as lf:
        df = lf.filter(pl.col("TIMESTAMP").is_not_null()).collect()
    assert len(df) == 2


def test_scan_tob_select(tob3_file: Path) -> None:
    with tob.scan_tob(tob3_file) as lf:
        df = lf.select(["TIMESTAMP", "A"]).collect()
    assert df.columns == ["TIMESTAMP", "A"]


def test_scan_tob_schema(tob3_file: Path) -> None:
    with tob.scan_tob(tob3_file) as lf:
        schema = lf.schema
    assert "TIMESTAMP" in schema
    assert schema["A"] == pl.Float32


def test_real_file() -> None:
    data_file = Path(__file__).parents[2] / "data" / "CS_121.dat"
    if not data_file.exists():
        pytest.skip("real data file not present")
    with tob.scan_tob(data_file) as lf:
        df = lf.collect()
    assert len(df) > 0
