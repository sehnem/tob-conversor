# tob_conversor

Convert Campbell Scientific **TOB1 / TOB2 / TOB3** binary logger files directly into
**pandas**, **polars**, or **DuckDB** DataFrames — with full type preservation, lazy
loading for large files, and a CLI for batch TOA5 / Parquet export.

---

## Features

| | |
|---|---|
| **Input formats** | TOB1 (5-line header), TOB2 / TOB3 (6-line header) — auto-detected |
| **Python engines** | `pandas`, `polars`, `duckdb` |
| **Lazy / chunked** | `scan_tob` (Polars lazy frame) and `read_tob_chunks` (pandas iterator) |
| **Schema introspection** | `read_header` — parse metadata without touching binary data |
| **Column types** | 21 Campbell Scientific types (IEEE4, FP2, INT, BOOL, ASCII, NSEC…) |
| **NaN handling** | Logger-model-aware FP2 thresholds; `None` / `NaN` in every engine |
| **Ring-buffer safety** | Unwritten TOB3 ring space is rejected, not decoded as data ([below](#tob3-is-a-ring-buffer)) |
| **Damaged card images** | A file that reads as empty is re-read for every run it holds ([below](#card-images-that-do-not-read-at-all)) |
| **Header encoding** | UTF-8 with a Latin-1 fallback, so `W/m²` and accented station names parse |
| **CLI output** | TOA5 ASCII (30-min split files) or Apache Parquet |
| **Parallel CLI** | Folder conversions use Rayon (`--jobs N`) |

### TOB3 is a ring buffer

A TOB3 file is **pre-allocated**: the logger reserves its whole *intended table
size* up front and overwrites frames in place. Everything past the write
pointer is not padding — it is whatever was on the card before, which on a
reused card means frames of another table, another program, or plain flash
garbage.

The only marker distinguishing a real frame is a 16-bit validation stamp in the
frame footer, so testing the stamp alone accepts about one junk frame in 2^17.
On a 2 GB card image that is roughly a dozen accepted junk frames, and each one
decodes its leading 12 bytes as seconds / subseconds / record number — yielding
a frame's worth of rows dated anywhere from **1990-01-01** (seconds ≈ 0, the
Campbell epoch) to **2126-02-12** (seconds ≈ 2³²). A single such row is enough
to ruin a min/max, a partition key, or an axis.

This reader therefore also requires that:

- the footer's **offset field fits inside the frame** and only appears together
  with the minor-frame flag;
- the frame's **`beg` record number continues the previous frame**; and
- the frame **begins in time where the previous one ended**, within a second of
  whole-second quantization plus a record interval. Junk that lines up on `beg`
  alone is not rare at two million frames a card, and this is what keeps a 1990
  or a 2040 run out of a 2025 file.

A frame at a genuine discontinuity (logger restart, ring wrap) is held until the
*next* frame corroborates it on both counts, so real data survives while junk —
which never lines up twice — does not.

Rejections are counted rather than hidden: `convert_streaming_with_stats`,
`TobBatchReader::frame_stats` and `tob.scan_frames(path)` report `frames_read`,
`frames_accepted`, `rejected_footer`, `rejected_unconfirmed`, the `frame_base`
the rows came from, and whether the file had to be `recovered`.
`frames_accepted == 0` is a verdict in its own right: the declared table was
never written to this card, as opposed to the file being unreadable.

### Card images that do not read at all

Everything above assumes frame 0 begins where the ASCII prolog ends, and that a
file holds one run. Both hold for a file a logger wrote; neither holds for
everything that reaches an archive. A card fragment can have its frames a fixed
number of bytes off, a prolog's CRLF padding can overlap frame 0, and a reused
card can hold two *instances* of the same table at two alignments — consecutive
in time, both real, and invisible to each other. Read at the wrong offset, every
footer lands mid-record and the file decodes to nothing.

So a file that yields **no rows at all** gets one second pass, and only then:
the reader scans it for runs — stretches of frames that corroborate each other
by record number and by the clock — also accepting the `val_stamp ± 1` footers
that CardConvert and "repair card" fragments carry, and then reads each run it
found, oldest first. Runs never overlap, so no row is read twice, and a pass
that produced rows is never replayed, so a healthy file behaves exactly as it
did before and never pays for any of this.

---

## Installation

### From source (development)

```bash
# 1. Install Rust (if not already present)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 2. Build and install the Python package into your active environment
cd tob_conversor_py
pip install maturin
maturin develop --release

# 3. Install optional dev dependencies (pytest, matplotlib, …)
pip install ".[dev]"
```

---

## Quick start

```python
import tob

# Inspect file metadata without reading binary data
h = tob.read_header("data/CS_121.dat")
print(h.station_name, h.logger_model, h.names)

# Eager load — choose your engine
df_pd  = tob.read_tob("data/CS_121.dat", engine="pandas")
df_pl  = tob.read_tob("data/CS_121.dat", engine="polars")
rel_db = tob.read_tob("data/CS_121.dat", engine="duckdb")

# Lazy load (Polars) — only the filtered rows are materialised
with tob.scan_tob("data/CS_120.dat") as lf:
    df = lf.filter(pl.col("TIMESTAMP") < cutoff).collect()

# Chunked iteration — process a 2 GB file without loading it all into RAM
for chunk in tob.read_tob_chunks("data/CS_120.dat", chunksize=65_536):
    process(chunk)  # chunk is a pandas DataFrame
```

---

## API reference

### Functions

| Function | Returns | Description |
|---|---|---|
| `read_header(path)` | `Header` | Parse the 6-line ASCII prolog; no binary I/O |
| `read_tob(path, *, engine, include_record, utc)` | `DataFrame` / `Relation` | Eager load into pandas, polars, or DuckDB |
| `scan_tob(path, *, include_record)` | `TobLazyFrame` | Lazy Polars frame backed by a temp Arrow IPC file |
| `read_tob_chunks(path, *, chunksize, include_record, utc)` | `Iterator[pd.DataFrame]` | Stream the file as fixed-size pandas chunks |
| `scan_frames(path)` | `FrameStats` | Walk the frames and report what they were; no measurements decoded |

### `FrameStats` attributes

| Attribute | Type | Description |
|---|---|---|
| `frames_read` | `int` | Main frames read from the file |
| `frames_accepted` | `int` | Frames emitted as data; `0` means the declared table was never written here |
| `rejected_footer` | `int` | Frames whose footer does not belong to this table |
| `rejected_unconfirmed` | `int` | Footer-valid frames no neighbour ever corroborated |
| `frame_base` | `int` | Byte offset the emitted frames were read from |
| `recovered` | `bool` | The rows came from the second pass over a damaged fragment |

### `Header` attributes

| Attribute | Type | Description |
|---|---|---|
| `station_name` | `str` | Logger station name |
| `logger_model` | `str` | Logger model (e.g. `"CR1000X"`) |
| `logger_sn` | `str` | Serial number |
| `table_name` | `str` | Data table name |
| `rec_intvl` | `float` | Record interval in seconds |
| `names` | `list[str]` | Column names |
| `units` | `list[str]` | Column units |
| `dtypes` | `list[str]` | Campbell Scientific type strings (e.g. `"FP2"`, `"IEEE4B"`) |
| `is_tob1` | `bool` | True for frameless TOB1 files |

### `TobLazyFrame`

A thin wrapper around `polars.LazyFrame` that owns the temporary Arrow IPC file.
Supports `.filter()`, `.select()`, `.with_columns()`, `.sort()`, `.limit()`, `.head()`,
`.collect()`, `.schema`, and context-manager cleanup.

```python
with tob.scan_tob("data/CS_120.dat") as lf:
    df = lf.filter(pl.col("w") > 0).select(["TIMESTAMP", "u", "v", "w"]).collect()
```

---

## Examples

| File | What it shows |
|---|---|
| [`examples/met_station.py`](examples/met_station.py) | Eager loading with pandas, polars, and DuckDB; cross-station hourly aggregation |
| [`examples/eddy_covariance.py`](examples/eddy_covariance.py) | Lazy loading of a 2 GB high-frequency file; first-minute slice; wind statistics |
| [`examples/quality_control.py`](examples/quality_control.py) | DuckDB SQL QC summary + Polars flag columns + matplotlib 3-panel time-series plot |
| [`examples/chunked_processing.py`](examples/chunked_processing.py) | Memory-efficient streaming of a 2 GB file via `read_tob_chunks`; online mean/std |

Run any example against the bundled data files:

```bash
python examples/quality_control.py
python examples/chunked_processing.py
```

---

## CLI (Rust binary)

The Rust crate ships a standalone `tobconversor` binary for batch conversion.
See [`tob_conversor_rs/README.md`](tob_conversor_rs/README.md) for full usage.

```bash
cargo build --release
# Convert a folder to TOA5 (30-min split files)
target/release/tobconversor data/ -o /tmp/toa5_out

# Convert to Parquet
target/release/tobconversor data/ -o /tmp/parquet_out --format parquet
```

---

## Development

```bash
# Build the Python extension
cd tob_conversor_py && maturin develop --release

# Python tests
pytest tob_conversor_py/tests/ -v

# Lint + format (Python)
ruff check tob_conversor_py/python/ examples/
ruff format tob_conversor_py/python/ examples/

# Rust tests, formatting, and linting
cargo test --manifest-path tob_conversor_rs/Cargo.toml
cargo fmt --manifest-path tob_conversor_rs/Cargo.toml
cargo clippy --manifest-path tob_conversor_rs/Cargo.toml -- -D warnings

# Install pre-commit hooks
pip install pre-commit && pre-commit install
```
