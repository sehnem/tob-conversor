//! Campbell Scientific **TOB1/TOB2/TOB3** binary logger files → TOA5, Parquet, or Arrow IPC.
//!
//! # Quick start
//! - High-level conversion: [`convert_streaming`] (TOA5), [`convert_tob_to_parquet`], or [`to_arrow_ipc_bytes`]
//! - Streaming batch reader: [`TobBatchReader`]
//! - Schema introspection: [`parse_tob_header`] → [`TobHeader`]
//! - Field-level decoding: [`decode_field_value`], [`format_field_toa5`], [`FieldValue`]
//! - Timestamp maths: [`time_ns_from_frame_header`], [`TO_EPOCH`]

pub mod tob;

pub use tob::{
    CsciType, FieldValue, TO_EPOCH, TobBatchReader, TobHeader, batch_to_ipc_stream_bytes,
    convert_streaming, convert_tob_to_parquet, decode_field_value, format_field_toa5,
    format_ns_timestamp, parse_tob_header, schema_to_ipc_stream_bytes, time_ns_from_frame_header,
    to_arrow_ipc_bytes, to_arrow_ipc_file,
};
