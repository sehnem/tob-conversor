# tobconversor

Convert Campbell Scientific **TOB1 / TOB2 / TOB3** binary logger files to
**TOA5** ASCII or **Parquet**.

---

## Table of contents

- [Features](#features)
- [CLI usage](#cli-usage)
- [Output format — TOA5](#output-format--toa5)
- [Library usage](#library-usage)
- [Project layout](#project-layout)
- [Building & testing](#building--testing)

---

## Features

| | |
|---|---|
| **Formats** | TOB1, TOB2, TOB3 (auto-detected from the 6-line ASCII header) |
| **Output** | TOA5 ASCII (default) or Apache Parquet (`--format parquet`) |
| **Splitting** | TOA5 output is split on 30-minute UTC boundaries by default |
| **Parallel** | Folder conversions use Rayon (`--jobs N`) |
| **RECORD column** | Optional logger record number column (`--record`) |
| **NaN** | Correctly quoted `"NAN"` per TOA5 spec; FP2 threshold is logger-model-aware |
| **Line endings** | CRLF (`\r\n`) throughout, as required by TOA5 |

---

## CLI usage

### Install / build

```bash
cargo build --release
# binary at: target/release/tobconversor
```

### Convert a folder

```bash
# Convert all .dat files in data/ to TOA5, output under data_converted/
tobconversor data/

# Explicit output directory
tobconversor data/ -o /tmp/toa5_out

# Include the RECORD column (logger record number)
tobconversor data/ -o /tmp/toa5_out --record

# Limit to 4 parallel threads
tobconversor data/ -o /tmp/toa5_out --jobs 4

# Single-threaded (useful for debugging)
tobconversor data/ -o /tmp/toa5_out --jobs 1
```

### Convert to Parquet

```bash
tobconversor data/ -o /tmp/parquet_out --format parquet
tobconversor data/CS_121.dat -o /tmp/parquet_out --format parquet --record
```

### Convert a single file

```bash
tobconversor data/CS_121.dat -o /tmp/toa5_out
```

### All options

```
Usage: tobconversor [OPTIONS] <INPUT>

Arguments:
  <INPUT>  Directory containing .dat files (searched recursively)

Options:
  -o, --output <OUTPUT>    Output directory [default: <input>_converted]
  -j, --jobs <JOBS>        Parallel worker threads; 0 = all CPUs [default: 0]
      --record             Include the RECORD column in the output
      --format <FORMAT>    Output format: toa5 (default) or parquet [default: toa5]
  -h, --help               Print help
```

---

## Output format — TOA5

Each `.dat` output file is a valid **TOA5** file: four header lines followed by
data rows, every line terminated with `\r\n`.

```
"TOA5","WeatherStation","CR1000X","12345","CR1000X.Std.01","MyProg.CR1X","9876","Hourly"
"TIMESTAMP","RECORD","Batt_V","AirTemp_C","RH"
"TS","RN","Volts","Deg C","%"
"","","Smp","Avg","Smp"
"2024-06-01 00:00:00",100,12.51,18.3,72.1
"2024-06-01 00:00:01",101,12.50,18.4,71.9
"2024-06-01 00:00:02",102,12.50,"NAN",71.8
```

**Splitting:** TOA5 files are split on 30-minute UTC boundaries. Each output
file is named `<stem>_<YYYY_MM_DD_HHmm>.dat`.

---

## Library usage

Add to your `Cargo.toml`:

```toml
[dependencies]
tobconversor = { path = "../tobconversor" }
```

### High-level: convert a file to TOA5

```rust
use tobconversor::convert_streaming;

// Writes split TOA5 .dat files into output_dir/
let files_written = convert_streaming(
    &input_path,
    &output_dir,
    30,    // split interval in minutes
    true,  // include RECORD column
)?;
println!("{files_written} output file(s) written");
```

### High-level: convert a file to Parquet

```rust
use tobconversor::convert_tob_to_parquet;

let files_written = convert_tob_to_parquet(&input_path, &output_dir, false)?;
```

### Schema introspection

```rust
use std::io::BufReader;
use std::fs::File;
use tobconversor::{parse_tob_header, TobHeader};

let f = File::open("data/CS_121.dat")?;
let mut buf = BufReader::new(f);
let header: TobHeader = parse_tob_header(&mut buf)?;

println!("Station : {}", header.station_name);
println!("Logger  : {}", header.logger_model);
println!("Table   : {}", header.table_name);
println!("Columns : {:?}", header.names);
println!("Units   : {:?}", header.units);
println!("Types   : {:?}", header.csci_dtypes);
println!("Rec size: {} bytes", header.line_nbytes);
```

### Field-level decoding (for custom sinks — e.g. a Python/PyO3 wrapper)

```rust
use tobconversor::{
    parse_tob_header, decode_field_value, format_field_toa5,
    time_ns_from_frame_header, FieldValue, TO_EPOCH,
};

// Decode one raw data field from its bytes:
let fv: FieldValue = decode_field_value(&header.csci_dtypes[0], &raw_bytes, header.fp2_nan);
match fv {
    FieldValue::F32(v)         => println!("float32: {v}"),
    FieldValue::F64(v)         => println!("float64: {v}"),
    FieldValue::I16(v)         => println!("int16:   {v}"),
    FieldValue::U16(v)         => println!("uint16:  {v}"),
    FieldValue::I32(v)         => println!("int32:   {v}"),
    FieldValue::U32(v)         => println!("uint32:  {v}"),
    FieldValue::Bool(v)        => println!("bool:    {v}"),
    FieldValue::TimestampNs(v) => println!("ts_ns:   {v}"),
    FieldValue::Str(s)         => println!("string:  {s}"),
    FieldValue::Null           => println!("NAN"),
}

// Or get a ready-to-write TOA5 cell string:
let cell: String = format_field_toa5(&header.csci_dtypes[0], &raw_bytes, header.fp2_nan);
// e.g. "12.5", "\"NAN\"", "\"sensor error\""

// Convert a raw frame timestamp to Unix nanoseconds:
let ns: i64 = time_ns_from_frame_header(seconds, subseconds, header.frame_time_res);

// Campbell epoch offset (seconds) — 1990-01-01 00:00:00 UTC:
println!("TO_EPOCH = {TO_EPOCH}");  // 631_152_000
```

---

## Project layout

| Path | Purpose |
|---|---|
| [src/lib.rs](src/lib.rs) | Crate root — public API re-exports |
| [src/main.rs](src/main.rs) | CLI (`clap`), parallel folder walk (`rayon`) |
| [src/tob/types.rs](src/tob/types.rs) | `CsciType` — 21 column types, byte sizes, Arrow schema mapping |
| [src/tob/header.rs](src/tob/header.rs) | 6-line ASCII header → `TobHeader`; FP2 NaN thresholds by model |
| [src/tob/decode.rs](src/tob/decode.rs) | Binary → `FieldValue` (typed) and TOA5 cell strings |
| [src/tob/subframe.rs](src/tob/subframe.rs) | TOB3 sub-frame boundary detection and validation |
| [src/tob/convert.rs](src/tob/convert.rs) | `convert_streaming` — TOA5 file writer with time-based splitting |
| [src/tob/parquet_writer.rs](src/tob/parquet_writer.rs) | `convert_tob_to_parquet` — columnar Parquet output |
| [tests/common/mod.rs](tests/common/mod.rs) | Synthetic frame/header builders shared by all integration tests |

---

## Building & testing

```bash
# Debug build
cargo build

# Optimised release build (LTO enabled)
cargo build --release

# All unit + integration tests
cargo test

# Single integration test file
cargo test --test convert_tob1

# Show println! output
cargo test -- --nocapture

# Optional smoke test with a real file
TOB3_TEST_FILE=data/CS_121.dat cargo test optional_real_file_smoke
```

### Test coverage

| File | What is tested |
|---|---|
| `tests/convert_tob1.rs` | TOB1 frameless records, epoch-zero timestamp, 30-min split, truncated EOF, `--record` |
| `tests/convert_tob2.rs` | TOB2 native frame decoding, 3-frame roundtrip |
| `tests/convert_classic_models.rs` | CR1000, CR1000X, CR10X roundtrips |
| `tests/convert_logger_models.rs` | CR800, CR3000, CR6; FP2 NaN threshold (CR10X vs CR1000) |
| `tests/convert_time_resolution.rs` | `Sec100Usec`, `SecUsec` frame time resolution |
| `tests/convert_edge_cases.rs` | Invalid frame skip, header-only file, 30-min split, IEEE4/BOOL columns, CRLF endings, `RECORD` column position |
| `tests/convert_real_file_smoke.rs` | Optional real `.dat` (skipped when env var unset) |

---

## Planned

- `--format parquet` — already implemented
- `tob3-python` — PyO3 wrapper crate: `read_tob(path) -> polars.DataFrame`
- `scan_tob` — lazy Polars source implementing `SerReader`
