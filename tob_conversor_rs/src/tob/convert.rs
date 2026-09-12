//! Streaming conversion: read TOB frames, emit TOA5 files.

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

use super::decode::{
    file_datetime_stamp, format_field_toa5, format_ns_timestamp, time_ns_from_frame_header,
};
use super::header::{TobHeader, parse_tob_header};
use super::subframe::{is_valid_main_frame, scan_and_skip_subframe_boundary};

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

fn emit_frame(frame_buf: &[u8], header: &TobHeader, out: &mut FrameOutput) -> Result<(), String> {
    let mut row_buf = String::with_capacity(256);
    let mut seconds = u32::from_le_bytes(frame_buf[0..4].try_into().unwrap());
    let mut subseconds = u32::from_le_bytes(frame_buf[4..8].try_into().unwrap());
    let main_beg = if header.is_tob2 {
        0
    } else {
        u32::from_le_bytes(frame_buf[8..12].try_into().unwrap())
    };

    let mut frame_time_ns = time_ns_from_frame_header(seconds, subseconds, header.frame_time_res);
    let mut next_record_id = main_beg;

    let data_end = header.frame_nbytes.saturating_sub(4);
    let line_step = header.line_nbytes + header.data_line_padding;
    let mut off = if header.is_tob2 { 8usize } else { 12usize };

    while off < data_end {
        if scan_and_skip_subframe_boundary(
            frame_buf,
            &mut off,
            data_end,
            header,
            next_record_id,
            &mut seconds,
            &mut subseconds,
            &mut frame_time_ns,
        ) {
            continue;
        }

        if off + header.line_nbytes > data_end {
            break;
        }

        let line_bytes = &frame_buf[off..off + header.line_nbytes];
        off += line_step;

        emit_one_row(
            line_bytes,
            header,
            frame_time_ns,
            next_record_id,
            out,
            &mut row_buf,
        )?;
        next_record_id = next_record_id.wrapping_add(1);
        frame_time_ns += (header.rec_intvl * 1_000_000_000.0) as i64;
    }

    Ok(())
}

pub fn convert_streaming(
    input: &Path,
    output_dir: &Path,
    split_interval_minutes: usize,
    include_record: bool,
) -> Result<usize, String> {
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

    if header.is_tob1 {
        let record_size = 12 + header.line_nbytes; // seconds(4) + ns(4) + record_id(4) + data
        let mut record_buf = vec![0u8; record_size];
        let mut row_buf = String::with_capacity(256);
        loop {
            match buff.read_exact(&mut record_buf) {
                Ok(()) => {}
                Err(_) => break, // EOF or truncated record at end of file
            }
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
        loop {
            match buff.read_exact(&mut frame_buf) {
                Ok(()) => {}
                Err(_) => break,
            }
            if !is_valid_main_frame(&frame_buf, &header) {
                continue;
            }
            emit_frame(&frame_buf, &header, &mut out)?;
        }
    }

    if let Some(mut w) = out.out_file.take() {
        let _ = w.flush();
    }

    Ok(out.written_files_count)
}
