"""Eddy-covariance data (CS_120.dat) — 100 ms, 17 IEEE4B columns.

Demonstrates lazy loading with polars: parse the header, inspect metadata, then
load only the first minute of data using a filter before collecting into memory.
"""

from __future__ import annotations

from pathlib import Path

import polars as pl

import tob

DATA = Path(__file__).parent.parent / "data" / "CS_120.dat"


def main() -> None:
    # --- Header inspection (no binary data read) ---
    h = tob.read_header(DATA)
    print(f"Station : {h.station_name}")
    print(f"Logger  : {h.logger_model}  SN={h.logger_sn}")
    print(f"Table   : {h.table_name}")
    print(f"Interval: {h.rec_intvl} s")
    print(f"Columns : {h.names}")
    print(f"Types   : {h.dtypes}")
    print()

    # --- Lazy load — slice first minute ---
    with tob.scan_tob(DATA) as lf:
        t_min = lf.select(pl.col("TIMESTAMP").min()).collect()["TIMESTAMP"][0]
        df = lf.filter(pl.col("TIMESTAMP") < t_min + pl.duration(minutes=1)).collect()

    print(f"Rows in first minute : {len(df)}")
    print(df.head())
    print()

    # --- Basic statistics on wind components ---
    stats = df.select(
        pl.col("u", "v", "w").mean().name.prefix("mean_"),
        pl.col("u", "v", "w").std().name.prefix("std_"),
    )
    print("Wind statistics:")
    print(stats)


if __name__ == "__main__":
    main()
