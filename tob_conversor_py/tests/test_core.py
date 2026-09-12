"""Tests for the PyO3 Header class and read_header function."""

from __future__ import annotations

from pathlib import Path

import pytest
import tob


def test_read_header_tob3(tob3_file: Path) -> None:
    h = tob.read_header(tob3_file)
    assert h.station_name == "ST"
    assert h.logger_model == "CR1000X"
    assert h.table_name == "T1"
    assert h.names == ["A", "B"]
    assert h.units == ["V", "V"]
    assert h.dtypes == ["Fp2", "Fp2"]
    assert not h.is_tob1
    assert not h.is_tob2


def test_read_header_tob1(tob1_file: Path) -> None:
    h = tob.read_header(tob1_file)
    assert h.is_tob1
    assert h.names == ["A", "B"]
    assert h.dtypes == ["Fp2", "Fp2"]


def test_read_header_rec_intvl(tob3_file: Path) -> None:
    h = tob.read_header(tob3_file)
    assert h.rec_intvl == pytest.approx(1.0)


def test_read_header_repr(tob3_file: Path) -> None:
    h = tob.read_header(tob3_file)
    r = repr(h)
    assert "ST" in r
    assert "CR1000X" in r
    assert "T1" in r


def test_read_header_missing_file(tmp_path: Path) -> None:
    with pytest.raises(OSError):
        tob.read_header(tmp_path / "nonexistent.dat")


def test_read_header_real_file() -> None:
    """Smoke test against the real CS_121.dat in the data/ folder."""
    data_file = Path(__file__).parents[2] / "data" / "CS_121.dat"
    if not data_file.exists():
        pytest.skip("real data file not present")
    h = tob.read_header(data_file)
    assert h.logger_model == "CR1000X"
    assert len(h.names) > 0
