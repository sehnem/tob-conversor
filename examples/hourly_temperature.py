"""Hourly mean sonic temperature for a selected week from the high-frequency EC data.

Scenario: CS_120.dat — 10 Hz (100 ms), 17 IEEE4B columns, CR1000X station.

26 million rows are aggregated inside DuckDB's vectorised engine; Python only
receives the 168-row hourly summary.
"""

from __future__ import annotations

from pathlib import Path

import tob

DATA = Path(__file__).parent.parent / "data" / "CS_120.dat"
WEEK_START = "2026-02-17"  # ISO date — first day of the selected week (UTC)
COLS = ["TIMESTAMP", "TS"]


def main() -> None:
    h = tob.read_header(DATA)
    print(f"Station  : {h.station_name or '—'}  ({h.logger_model})")
    print(f"Table    : {h.table_name}  ·  {len(h.names)} columns total")
    print(f"Week     : {WEEK_START}  →  +7 days  (loading {len(COLS)} columns)")
    print()

    with tob.open_tob(DATA, columns=COLS) as conn:
        df = conn.sql(
            f"""
            SELECT
                date_trunc('hour', TIMESTAMP) AS hour,
                avg(TS)                        AS temp_mean,
                min(TS)                        AS temp_min,
                max(TS)                        AS temp_max,
                count(*)                       AS n_samples,
            FROM tob
            WHERE TIMESTAMP >= TIMESTAMPTZ '{WEEK_START}'
              AND TIMESTAMP <  TIMESTAMPTZ '{WEEK_START}'::DATE + INTERVAL 7 DAY
            GROUP BY 1
            ORDER BY 1
            """
        ).df()

    if df.empty:
        print(f"No data found for week starting {WEEK_START}.")
        return

    header = f"{'Hour (UTC)':<25}  {'Mean °C':>8}  {'Min °C':>7}  {'Max °C':>7}  {'Samples':>10}"
    print(header)
    print("-" * len(header))
    for _, row in df.iterrows():
        print(
            f"{str(row['hour']):<25}  "
            f"{row['temp_mean']:>8.3f}  "
            f"{row['temp_min']:>7.3f}  "
            f"{row['temp_max']:>7.3f}  "
            f"{int(row['n_samples']):>10,}"
        )

    print()
    week_mean = df["temp_mean"].mean()
    week_min = df["temp_min"].min()
    week_max = df["temp_max"].max()
    print(
        f"{len(df)} hourly rows  ·  week mean {week_mean:.3f} °C  "
        f"·  range [{week_min:.3f}, {week_max:.3f}] °C"
    )


if __name__ == "__main__":
    main()
