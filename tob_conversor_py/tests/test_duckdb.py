"""Tests for the DuckDB engine."""

from __future__ import annotations

from pathlib import Path

import duckdb
import pytest
import tob


def test_returns_relation(tob3_file: Path) -> None:
    rel = tob.read_tob(tob3_file, engine="duckdb")
    assert isinstance(rel, duckdb.DuckDBPyRelation)


def test_row_count(tob3_file: Path) -> None:
    rel = tob.read_tob(tob3_file, engine="duckdb")
    row = rel.count("*").fetchone()
    assert row is not None
    assert row[0] == 2


def test_column_names(tob3_file: Path) -> None:
    rel = tob.read_tob(tob3_file, engine="duckdb")
    assert "TIMESTAMP" in rel.columns
    assert "A" in rel.columns


def test_sql_query(tob3_file: Path) -> None:
    rel = tob.read_tob(tob3_file, engine="duckdb")
    result = rel.filter("A > 10").fetchdf()
    assert len(result) == 2


def test_to_pandas(tob3_file: Path) -> None:
    rel = tob.read_tob(tob3_file, engine="duckdb")
    df = rel.df()
    assert len(df) == 2


def test_real_file() -> None:
    data_file = Path(__file__).parents[2] / "data" / "CS_121.dat"
    if not data_file.exists():
        pytest.skip("real data file not present")
    rel = tob.read_tob(data_file, engine="duckdb")
    row = rel.count("*").fetchone()
    assert row is not None
    assert row[0] > 0
