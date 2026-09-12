"""Tests for the pandas engine."""

from __future__ import annotations

from pathlib import Path

import numpy as np
import pandas as pd
import pytest
import tob


def test_returns_dataframe(tob3_file: Path) -> None:
    df = tob.read_tob(tob3_file, engine="pandas")
    assert isinstance(df, pd.DataFrame)


def test_row_count(tob3_file: Path) -> None:
    df = tob.read_tob(tob3_file, engine="pandas")
    assert len(df) == 2


def test_column_names(tob3_file: Path) -> None:
    df = tob.read_tob(tob3_file, engine="pandas")
    assert "TIMESTAMP" in df.columns
    assert "A" in df.columns
    assert "B" in df.columns


def test_timestamp_dtype_utc(tob3_file: Path) -> None:
    df = tob.read_tob(tob3_file, engine="pandas")
    assert str(df["TIMESTAMP"].dtype) == "datetime64[ns, UTC]"


def test_timestamp_dtype_naive(tob3_file: Path) -> None:
    df = tob.read_tob(tob3_file, engine="pandas", utc=False)
    assert df["TIMESTAMP"].dtype == np.dtype("datetime64[ns]")


def test_float32_dtype(tob3_file: Path) -> None:
    df = tob.read_tob(tob3_file, engine="pandas")
    assert df["A"].dtype == np.dtype("float32")
    assert df["B"].dtype == np.dtype("float32")


def test_fp2_value(tob3_file: Path) -> None:
    df = tob.read_tob(tob3_file, engine="pandas")
    assert df["A"].iloc[0] == pytest.approx(14.22, abs=1e-4)


def test_null_propagation(tob3_file_with_nan: Path) -> None:
    df = tob.read_tob(tob3_file_with_nan, engine="pandas")
    assert not pd.isna(df["A"].iloc[0])
    assert pd.isna(df["B"].iloc[0])


def test_include_record(tob3_file: Path) -> None:
    df = tob.read_tob(tob3_file, engine="pandas", include_record=True)
    assert "RECORD" in df.columns


def test_real_file() -> None:
    data_file = Path(__file__).parents[2] / "data" / "CS_121.dat"
    if not data_file.exists():
        pytest.skip("real data file not present")
    df = tob.read_tob(data_file, engine="pandas")
    assert len(df) > 0
    assert str(df["TIMESTAMP"].dtype) == "datetime64[ns, UTC]"
