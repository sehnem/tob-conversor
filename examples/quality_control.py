"""Quality control and visualization for high-frequency eddy-covariance data.

Scenario: CS_120.dat — 10 Hz (100 ms), 17 IEEE4B columns, CR1000X station.

tob.open_tob converts the TOB binary to Parquet (Rust) and registers a DuckDB
view over only the requested columns — the remaining 13 are never read.
QC filtering and 1-hour aggregation happen inside DuckDB's vectorised engine;
only the small aggregated result reaches Python for plotting.
"""

from __future__ import annotations

from pathlib import Path

import matplotlib.dates as mdates
import matplotlib.pyplot as plt
import tob

DATA = Path(__file__).parent.parent / "data" / "CS_120.dat"
COLS = ["TIMESTAMP", "w", "diag_CSAT", "Diag_LI7500"]


def main() -> None:
    h = tob.read_header(DATA)
    print(f"Station  : {h.station_name}  ({h.logger_model})")
    print(f"Columns  : loading {len(COLS)} of {len(h.names)}  →  {COLS}")
    print()

    with tob.open_tob(DATA, columns=COLS) as conn:
        df = conn.sql(
            """
            SELECT
                date_trunc('hour', TIMESTAMP)                             AS hour,
                avg(w)                                                    AS w_raw,
                avg(w) FILTER (WHERE diag_CSAT = 0 AND Diag_LI7500 = 0) AS w_qc,
            FROM tob
            GROUP BY 1
            ORDER BY 1
            """
        ).df()

    print(f"{len(df)} hours  ·  {df['w_qc'].notna().sum()} with QC data")

    fig, ax = plt.subplots(figsize=(13, 4))
    ax.plot(df["hour"], df["w_raw"], color="lightsteelblue", linewidth=0.9, label="Raw")
    ax.plot(
        df["hour"],
        df["w_qc"],
        color="steelblue",
        linewidth=1.1,
        label="QC-filtered  (diag_CSAT = 0  ∧  Diag_LI7500 = 0)",
    )
    ax.set_ylabel("Vertical wind  w  (m/s)", fontsize=10)
    ax.set_title(
        f"CS_120 — Vertical wind, 1-h mean  "
        f"(10 Hz source · {len(COLS)}/{len(h.names)} columns loaded)",
        fontsize=11,
    )
    ax.xaxis.set_major_formatter(mdates.DateFormatter("%Y-%m-%d"))
    ax.xaxis.set_major_locator(mdates.AutoDateLocator())
    fig.autofmt_xdate(rotation=30)
    ax.legend(fontsize=9)
    ax.grid(True, alpha=0.25)
    plt.tight_layout()

    out = Path(__file__).with_suffix(".png")
    plt.savefig(out, dpi=150, bbox_inches="tight")
    print(f"Plot saved → {out}")
    plt.show()


if __name__ == "__main__":
    main()
