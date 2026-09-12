"""Meteorological stations (CS_121.dat, CS_131.dat) — 1 min, FP2 + IEEE4B.

Demonstrates:
  - Eager loading with pandas and polars
  - Cross-station comparison with DuckDB (querying pandas DataFrames directly)
"""

from __future__ import annotations

from pathlib import Path

import tob

DATA = Path(__file__).parent.parent / "data"
CS121 = DATA / "CS_121.dat"
CS131 = DATA / "CS_131.dat"


def pandas_example() -> None:
    print("=== pandas ===")
    df = tob.read_tob(CS121, engine="pandas")
    print(df.dtypes)
    print()
    print(df[["TIMESTAMP", "AirTC", "RH", "Pressao"]].head())
    print()


def polars_example() -> None:
    print("=== polars ===")
    df = tob.read_tob(CS121, engine="polars")
    print(df.schema)
    print()
    print(df.select(["TIMESTAMP", "AirTC", "RH", "Pressao"]).head())
    print()


def duckdb_example() -> None:
    import duckdb

    print("=== duckdb — hourly aggregations across stations ===")
    conn = duckdb.connect()
    conn.register("cs121", tob.read_tob(CS121, engine="pandas"))
    conn.register("cs131", tob.read_tob(CS131, engine="pandas"))

    # Stack both stations with a label and compute hourly mean temperature.
    result = conn.sql(
        """
        SELECT
            station,
            date_trunc('hour', TIMESTAMP) AS hour,
            round(avg(AirTC), 2)          AS mean_AirTC,
            round(avg(RH), 1)             AS mean_RH
        FROM (
            SELECT 'CS_121' AS station, TIMESTAMP, AirTC, RH FROM cs121
            UNION ALL
            SELECT 'CS_131' AS station, TIMESTAMP, AirTC, RH FROM cs131
        )
        GROUP BY station, hour
        ORDER BY station, hour
        LIMIT 10
        """
    )
    print(result)
    print()


def main() -> None:
    pandas_example()
    polars_example()
    duckdb_example()


if __name__ == "__main__":
    main()
