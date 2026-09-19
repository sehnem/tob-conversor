"""Memory-efficient processing of a large TOB file (CS_120.dat — 2 GB).

Demonstrates:
  - read_tob_chunks: iterates the file in fixed-size pandas DataFrames without
    loading everything into RAM at once; suitable for files larger than available
    memory
  - Online (streaming) mean and standard deviation via sum / sum-of-squares
    accumulators — no full-data materialisation required
"""

from __future__ import annotations

from pathlib import Path

import numpy as np
import tob

DATA = Path(__file__).parent.parent / "data" / "CS_120.dat"
WIND_COLS = ["u", "v", "w"]
CHUNK = 65_536  # rows per chunk (~6.5 seconds of 100 Hz data)


def main() -> None:
    h = tob.read_header(DATA)
    hz = 1.0 / h.rec_intvl if h.rec_intvl else float("nan")
    print(f"File     : {DATA.name}")
    print(f"Station  : {h.station_name}  ({h.logger_model})")
    print(f"Table    : {h.table_name}")
    print(f"Interval : {h.rec_intvl} s  ({hz:.0f} Hz)")
    print(f"Columns  : {h.names}")
    print()

    # Accumulators for each wind component
    n: dict[str, int] = {col: 0 for col in WIND_COLS}
    s1: dict[str, float] = {col: 0.0 for col in WIND_COLS}  # Σx
    s2: dict[str, float] = {col: 0.0 for col in WIND_COLS}  # Σx²

    total_rows = 0
    for chunk in tob.read_tob_chunks(DATA, chunksize=CHUNK):
        total_rows += len(chunk)
        for col in WIND_COLS:
            arr = chunk[col].dropna().to_numpy(dtype="float64")
            if len(arr) == 0:
                continue
            n[col] += len(arr)
            s1[col] += arr.sum()
            s2[col] += float(np.dot(arr, arr))

        print(f"\r  rows processed: {total_rows:>12,}", end="", flush=True)

    print(f"\n\nTotal rows : {total_rows:,}")
    print()
    print(f"{'Column':<8}  {'Mean':>10}  {'Std':>10}  {'N':>12}")
    print("-" * 46)
    for col in WIND_COLS:
        if n[col] == 0:
            print(f"{col:<8}  {'N/A':>10}  {'N/A':>10}  {0:>12,}")
            continue
        mean = s1[col] / n[col]
        var = max(s2[col] / n[col] - mean**2, 0.0)
        print(f"{col:<8}  {mean:>10.4f}  {var**0.5:>10.4f}  {n[col]:>12,}")


if __name__ == "__main__":
    main()
