//! TOB1/TOB2/TOB3 binary format: header parsing, field decoding, frame walking, TOA5/Parquet/IPC export.

mod convert;
pub mod decode;
pub mod frame_gate;
mod header;
pub mod ipc_writer;
pub mod parquet_writer;
mod subframe;
mod types;

pub use convert::{convert_streaming, convert_streaming_with_stats};
pub use decode::{
    FieldValue, TO_EPOCH, decode_field_value, format_field_toa5, format_ns_timestamp,
    time_ns_from_frame_header,
};
pub use frame_gate::{FrameStats, is_valid_main_frame};
pub use header::{TobHeader, parse_tob_header};
pub use ipc_writer::{
    TobBatchReader, batch_to_ipc_stream_bytes, schema_to_ipc_stream_bytes, to_arrow_ipc_bytes,
    to_arrow_ipc_file,
};
pub use parquet_writer::convert_tob_to_parquet;
pub use types::CsciType;
