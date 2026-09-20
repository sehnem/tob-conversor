//! Campbell Scientific **TOB1/TOB2/TOB3** binary logger files → TOA5, Parquet, or Arrow IPC.
//!
//! # Quick start
//! - High-level conversion: [`convert_streaming`] (TOA5), [`convert_tob_to_parquet`], or [`to_arrow_ipc_bytes`]
//! - Streaming batch reader: [`TobBatchReader`]
//! - Schema introspection: [`parse_tob_header`] → [`TobHeader`]
//! - Frame validation for ring files: [`is_valid_main_frame`], [`FrameStats`], [`scan_frame_stats`]
//! - Field-level decoding: [`decode_field_value`], [`format_field_toa5`], [`FieldValue`]
//! - Timestamp maths: [`time_ns_from_frame_header`], [`TO_EPOCH`]

pub mod tob;

pub use tob::{
    CsciType, FieldValue, FrameStats, FrameWalk, StampRule, TO_EPOCH, TobBatchReader, TobHeader,
    batch_to_ipc_stream_bytes, convert_streaming, convert_streaming_with_stats,
    convert_tob_to_parquet, decode_field_value, format_field_toa5, format_ns_timestamp,
    is_valid_main_frame, is_valid_main_frame_with, parse_tob_header, scan_frame_stats,
    schema_to_ipc_stream_bytes, time_ns_from_frame_header, to_arrow_ipc_bytes, to_arrow_ipc_file,
};
