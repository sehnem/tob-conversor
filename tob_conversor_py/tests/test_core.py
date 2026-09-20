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


def test_scan_frames_counts_a_clean_file(tob3_file: Path) -> None:
    stats = tob.scan_frames(tob3_file)
    assert stats.frames_read == 2
    assert stats.frames_accepted == 2
    assert stats.rejected_footer == 0
    assert stats.rejected_unconfirmed == 0
    assert not stats.recovered


def test_scan_frames_reports_a_card_with_no_run_as_empty(tob3_junk_only_file: Path) -> None:
    """EX-5: "this table was never written here" is a verdict, not a failure.

    The file parses, has frames, and holds nothing — which used to be
    indistinguishable from "not this table" without scanning it again.
    """
    stats = tob.scan_frames(tob3_junk_only_file)
    assert stats.frames_read > 0
    assert stats.frames_accepted == 0
    # No invented rows: the stamp collisions in the file do not become data.
    assert len(tob.read_tob(tob3_junk_only_file)) == 0


def test_off_by_one_stamps_are_recovered(tob3_off_by_one_file: Path) -> None:
    """EX-1: a fragment whose footers carry ``val_stamp + 1`` still reads."""
    df = tob.read_tob(tob3_off_by_one_file)
    assert len(df) == 4
    stats = tob.scan_frames(tob3_off_by_one_file)
    assert stats.frames_accepted == 4
    assert stats.recovered, "this file only reads on the second pass"


def test_reader_frame_stats_match_the_scan(tob3_file: Path) -> None:
    from tob._core import TobReader

    reader = TobReader(str(tob3_file), False, 65536)
    for _ in reader:
        pass
    stats = reader.frame_stats
    assert stats.frames_accepted == tob.scan_frames(tob3_file).frames_accepted
    assert "frames_accepted=2" in repr(stats)
