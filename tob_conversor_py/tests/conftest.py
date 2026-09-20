"""Synthetic TOB binary fixtures.

Binary layouts mirror tob_conversor_rs/tests/common/mod.rs so the same
frame structures are tested end-to-end through the Python wrappers.
"""

from __future__ import annotations

import struct
from pathlib import Path

import pytest

# Matches VAL_STAMP used in the Rust test helpers.
VAL_STAMP: int = 4660  # 0x1234

# FP2 bytes that decode to 14.22:
#   big-endian i16 = 0x458E = 17806
#   sign=0, exponent=2, mantissa=(17806 & 0x1FFF)=1422, value=1422*1e-2=14.22
_FP2_14_22 = bytes([0x45, 0x8E])

# FP2 bytes that hit the CR1000X NaN threshold (7999 >= 7999.0):
#   big-endian u16 = 7999 = 0x1F3F
#   sign=0, exponent=0, mantissa=7999, value=7999.0 → NaN
_FP2_NAN = bytes([0x1F, 0x3F])


def _tob3_header(frame_nbytes: int = 20, val_stamp: int = VAL_STAMP) -> bytes:
    lines = "\n".join(
        [
            '"TOB3","ST","CR1000X","123","OS","PROG","SIG","2020-01-01 00:00:00"',
            f'"T1","1 SEC","{frame_nbytes}","0","{val_stamp}","SecMsec","0","0","0"',
            '"A","B"',
            '"V","V"',
            '"Smp","Smp"',
            '"FP2","FP2"',
            "",
        ]
    )
    return lines.encode()


def _tob3_frame(
    seconds: int,
    beg_rec: int,
    payload: bytes = _FP2_14_22 + _FP2_14_22,
    val_stamp: int = VAL_STAMP,
) -> bytes:
    header = struct.pack("<III", seconds, 0, beg_rec)
    footer = struct.pack("<I", val_stamp << 16)
    return header + payload + footer


def _tob1_header() -> bytes:
    # TOB1 has a five-line prolog: no table/frame-geometry line, and the table
    # name lives on the environment line. Binary records follow immediately.
    lines = "\n".join(
        [
            '"TOB1","ST","CR1000X","123","OS","PROG","SIG","T1"',
            '"SECONDS","NANOSECONDS","RECORD","A","B"',
            '"Seconds","Nanoseconds","","V","V"',
            '"","","","Smp","Smp"',
            '"ULONG","ULONG","ULONG","FP2","FP2"',
            "",
        ]
    )
    return lines.encode()


def _tob1_record(seconds: int, nanoseconds: int, record_id: int, data: bytes) -> bytes:
    return struct.pack("<III", seconds, nanoseconds, record_id) + data


@pytest.fixture()
def tob3_file(tmp_path: Path) -> Path:
    """TOB3 file with 2 FP2 columns (A=14.22, B=14.22), 2 frames."""
    data = _tob3_header() + _tob3_frame(100, 1) + _tob3_frame(101, 2)
    path = tmp_path / "test.dat"
    path.write_bytes(data)
    return path


@pytest.fixture()
def tob3_file_with_nan(tmp_path: Path) -> Path:
    """TOB3 file where column B of the first row hits the FP2 NaN threshold."""
    payload = _FP2_14_22 + _FP2_NAN
    data = _tob3_header() + _tob3_frame(100, 1, payload=payload)
    path = tmp_path / "nan.dat"
    path.write_bytes(data)
    return path


@pytest.fixture()
def tob3_off_by_one_file(tmp_path: Path) -> Path:
    """A card fragment whose main footers carry ``val_stamp + 1``.

    CardConvert and "repair card" write these, and read strictly the file is
    empty.  Four frames, so the run is long enough to be more than a
    coincidence, with record numbers and a 1 SEC clock that agree.
    """
    frames = b"".join(_tob3_frame(100 + i, 1 + i, val_stamp=VAL_STAMP + 1) for i in range(4))
    path = tmp_path / "offbyone.dat"
    path.write_bytes(_tob3_header() + frames)
    return path


@pytest.fixture()
def tob3_junk_only_file(tmp_path: Path) -> Path:
    """A card whose declared table was never written to it.

    A handful of footer stamps collide by chance — one junk frame in 2^17 does
    — but nothing continues anything, so there are no rows to be had.
    """
    frames = []
    for i in range(40):
        if i and i % 9 == 0:
            frames.append(_tob3_frame(0xDEAD_0000 + i, 0xABCD_0000 ^ i))
        else:
            frames.append(_tob3_frame(i, i, val_stamp=VAL_STAMP ^ 0x0F0F))
    path = tmp_path / "junk.dat"
    path.write_bytes(_tob3_header() + b"".join(frames))
    return path


@pytest.fixture()
def tob1_file(tmp_path: Path) -> Path:
    """TOB1 file with 2 FP2 columns, 2 records."""
    data = (
        _tob1_header()
        + _tob1_record(100, 0, 1, _FP2_14_22 + _FP2_14_22)
        + _tob1_record(101, 0, 2, _FP2_14_22 + _FP2_14_22)
    )
    path = tmp_path / "tob1.dat"
    path.write_bytes(data)
    return path
