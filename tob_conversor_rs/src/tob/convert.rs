//! Streaming conversion: read TOB frames, emit TOA5 files.

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

use super::decode::{
    file_datetime_stamp, format_field_toa5, format_ns_timestamp, time_ns_from_frame_header,
};
use super::frame_gate::{Admit, FrameGate, FrameStats};
use super::header::{TobHeader, parse_tob_header};
use super::subframe::walk_frame;

struct FrameOutput<'a> {
    current_interval: i64,
    out_file: Option<BufWriter<File>>,
    written_files_count: usize,
    interval_ns: i64,
    full_ascii_header: &'a str,
    output_dir: &'a Path,
    base_name: &'a str,
    include_record: bool,
}

fn emit_one_row(
    line_bytes: &[u8],
    header: &TobHeader,
    frame_time_ns: i64,
    record_id: u32,
    out: &mut FrameOutput,
    row_buf: &mut String,
) -> Result<(), String> {
    let file_time_idx = if out.interval_ns > 0 {
        frame_time_ns / out.interval_ns
    } else {
        0
    };

    if file_time_idx != out.current_interval {
        out.current_interval = file_time_idx;
        out.written_files_count += 1;

        if let Some(mut w) = out.out_file.take() {
            let _ = w.flush();
        }

        let stamp = file_datetime_stamp(frame_time_ns);
        let out_name = format!("{}_{}.dat", out.base_name, stamp);
        let out_path = out.output_dir.join(&out_name);

        let df = File::create(&out_path).map_err(|e| e.to_string())?;
        let mut w = BufWriter::new(df);
        w.write_all(out.full_ascii_header.as_bytes())
            .map_err(|e| e.to_string())?;
        out.out_file = Some(w);
    }

    if let Some(writer) = out.out_file.as_mut() {
        row_buf.clear();
        row_buf.push_str(&format_ns_timestamp(frame_time_ns));
        if out.include_record {
            row_buf.push(',');
            row_buf.push_str(&record_id.to_string());
        }
        let mut offset = 0;
        for dt in &header.csci_dtypes {
            let sz = dt.size();
            row_buf.push(',');
            row_buf.push_str(&format_field_toa5(
                dt,
                &line_bytes[offset..offset + sz],
                header.fp2_nan,
            ));
            offset += sz;
        }
        write!(writer, "{}\r\n", row_buf).map_err(|e| e.to_string())?;
    }

    Ok(())
}

fn emit_frame(
    frame_buf: &[u8],
    header: &TobHeader,
    out: &mut FrameOutput,
) -> Result<usize, String> {
    let mut row_buf = String::with_capacity(256);
    let mut failure: Option<String> = None;

    let rows = walk_frame(frame_buf, header, |line_bytes, frame_time_ns, record_id| {
        if failure.is_some() {
            return;
        }
        if let Err(e) = emit_one_row(
            line_bytes,
            header,
            frame_time_ns,
            record_id,
            out,
            &mut row_buf,
        ) {
            failure = Some(e);
        }
    });

    match failure {
        Some(e) => Err(e),
        None => Ok(rows),
    }
}

/// Convert a TOB file to TOA5 files in `output_dir`, returning the number of
/// files written.
pub fn convert_streaming(
    input: &Path,
    output_dir: &Path,
    split_interval_minutes: usize,
    include_record: bool,
) -> Result<usize, String> {
    convert_streaming_with_stats(input, output_dir, split_interval_minutes, include_record)
        .map(|(files, _)| files)
}

/// As [`convert_streaming`], but also reports how many main frames were read
/// and how many were rejected as not belonging to this table — the unwritten
/// tail of a pre-allocated TOB3 ring usually dwarfs the real data, and a caller
/// recording provenance wants that number rather than a silent difference.
pub fn convert_streaming_with_stats(
    input: &Path,
    output_dir: &Path,
    split_interval_minutes: usize,
    include_record: bool,
) -> Result<(usize, FrameStats), String> {
    let file = File::open(input).map_err(|e| format!("Open error: {}", e))?;
    let mut buff = BufReader::new(file);

    let header = parse_tob_header(&mut buff).map_err(|e| format!("Header parse error: {}", e))?;

    let base_name = input.file_stem().unwrap().to_string_lossy().to_string();
    std::fs::create_dir_all(output_dir).map_err(|e| e.to_string())?;

    let interval_ns = (split_interval_minutes as i64) * 60 * 1_000_000_000;

    let l1 = format!(
        "\"TOA5\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\"",
        header.station_name,
        header.logger_model,
        header.logger_sn,
        header.logger_os,
        header.logger_program,
        header.logger_program_signature,
        header.table_name
    );

    let mut head_names = vec!["\"TIMESTAMP\"".to_string()];
    let mut head_units = vec!["\"TS\"".to_string()];
    let mut head_procs = vec!["\"\"".to_string()];
    if include_record {
        head_names.push("\"RECORD\"".to_string());
        head_units.push("\"RN\"".to_string());
        head_procs.push("\"\"".to_string());
    }

    for i in 0..header.names.len() {
        head_names.push(format!("\"{}\"", header.names[i]));
        head_units.push(format!("\"{}\"", header.units[i]));
        head_procs.push(format!("\"{}\"", header.processing[i]));
    }
    let full_ascii_header = format!(
        "{}\r\n{}\r\n{}\r\n{}\r\n",
        l1,
        head_names.join(","),
        head_units.join(","),
        head_procs.join(",")
    );

    let mut out = FrameOutput {
        current_interval: -1,
        out_file: None,
        written_files_count: 0,
        interval_ns,
        full_ascii_header: &full_ascii_header,
        output_dir,
        base_name: &base_name,
        include_record,
    };

    let mut frame_stats = FrameStats::default();

    if header.is_tob1 {
        let record_size = 12 + header.line_nbytes; // seconds(4) + ns(4) + record_id(4) + data
        let mut record_buf = vec![0u8; record_size];
        let mut row_buf = String::with_capacity(256);
        // A short read means EOF or a truncated trailing record; either way the
        // file is done.
        while buff.read_exact(&mut record_buf).is_ok() {
            let seconds = u32::from_le_bytes(record_buf[0..4].try_into().unwrap());
            let subseconds = u32::from_le_bytes(record_buf[4..8].try_into().unwrap());
            let record_id = u32::from_le_bytes(record_buf[8..12].try_into().unwrap());
            let frame_time_ns =
                time_ns_from_frame_header(seconds, subseconds, header.frame_time_res);
            emit_one_row(
                &record_buf[12..],
                &header,
                frame_time_ns,
                record_id,
                &mut out,
                &mut row_buf,
            )?;
        }
    } else {
        if header.frame_nbytes == 0 {
            return Err("frame_nbytes is 0 in non-TOB1 file".to_string());
        }
        let mut frame_buf = vec![0u8; header.frame_nbytes];
        let mut gate = FrameGate::new();
        while buff.read_exact(&mut frame_buf).is_ok() {
            match gate.admit(&frame_buf, &header, |f| {
                walk_frame(f, &header, |_, _, _| {})
            }) {
                Admit::Skip => continue,
                Admit::Current => {
                    let rows = emit_frame(&frame_buf, &header, &mut out)?;
                    gate.advance(rows);
                }
                Admit::HeldThenCurrent => {
                    if let Some(held) = gate.take_held() {
                        emit_frame(&held, &header, &mut out)?;
                    }
                    let rows = emit_frame(&frame_buf, &header, &mut out)?;
                    gate.advance(rows);
                }
            }
        }
        gate.finish();
        frame_stats = gate.stats();
    }

    if let Some(mut w) = out.out_file.take() {
        let _ = w.flush();
    }

    Ok((out.written_files_count, frame_stats))
}
