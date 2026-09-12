# tob_conversor

Convert Campbell Scientific **TOB1 / TOB2 / TOB3** binary logger files directly into
**pandas**, **polars**, or **DuckDB** DataFrames — with full type preservation, lazy
loading for large files, and a CLI for batch TOA5 / Parquet export.

---

## Features

| | |
|---|---|
| **Input formats** | TOB1, TOB2, TOB3 (auto-detected from the 6-line ASCII header) |
| **Python engines** | `pandas`, `polars`, `duckdb` |
| **Lazy / chunked** | `scan_tob` (Polars lazy frame) and `read_tob_chunks` (pandas iterator) |
| **Schema introspection** | `read_header` — parse metadata without touching binary data |
| **Column types** | 21 Campbell Scientific types (IEEE4, FP2, INT, BOOL, ASCII, NSEC…) |
| **NaN handling** | Logger-model-aware FP2 thresholds; `None` / `NaN` in every engine |
| **CLI output** | TOA5 ASCII (30-min split files) or Apache Parquet |
| **Parallel CLI** | Folder conversions use Rayon (`--jobs N`) |

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
