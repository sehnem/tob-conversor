//! ASCII TOB file header (first 6 lines) → [`TobHeader`].

use std::io::BufRead;

use super::types::CsciType;

/// Parsed metadata from the 6-line TOB prolog.
#[derive(Debug, Clone)]
pub struct TobHeader {
    pub is_tob1: bool,
    pub is_tob2: bool,
    pub station_name: String,
    pub logger_model: String,
    pub logger_sn: String,
    pub logger_os: String,
    pub logger_program: String,
    pub logger_program_signature: String,
    pub table_name: String,
    pub rec_intvl: f64,
    pub frame_nbytes: usize,
    pub frame_time_res: f64,
    pub names: Vec<String>,
    pub units: Vec<String>,
    pub processing: Vec<String>,
    pub csci_dtypes: Vec<CsciType>,
    /// Max. full rows that fit in the frame data segment (`frame_nbytes - 16` / `line_nbytes`).
    /// Actual emitted rows follow **sub-frame** boundaries in `convert`.
    #[allow(dead_code)]
    pub data_nlines: usize,
    pub line_nbytes: usize,
    pub data_line_padding: usize,
    pub val_stamp: u16,
    pub comp_val_stamp: u16,
    pub fp2_nan: f32,
}

/// FP2 “infinity” threshold depends on logger family (camp2ascii-style).
pub(crate) fn fp2_nan_threshold(logger_model: &str) -> f32 {
    if logger_model.starts_with("CR1000") {
        7999.0
    } else if logger_model.starts_with("CR10") {
        6999.0
    } else {
        7999.0
    }
}

pub(crate) fn split_csv_line(line: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for c in line.chars() {
        if c == '"' {
            in_quotes = !in_quotes;
        } else if c == ',' && !in_quotes {
            result.push(current.trim_matches('"').to_string());
            current.clear();
        } else {
            current.push(c);
        }
    }
    result.push(current.trim().trim_matches('"').to_string());
    result
}

/// Read one header line, tolerating the Latin-1 that Campbell loggers write.
///
/// Unit strings like `W/m²` and Portuguese station names arrive as single
/// high bytes, which are not valid UTF-8.  Decoding strictly rejects the whole
/// file — so try UTF-8 first (modern exports) and fall back to Latin-1, where
/// every byte maps straight to the code point of the same value.
fn read_header_line(buff: &mut impl BufRead) -> std::io::Result<Option<String>> {
    let mut raw: Vec<u8> = Vec::new();
    let n = buff.read_until(b'\n', &mut raw)?;
    if n == 0 {
        return Ok(None);
    }
    Ok(Some(match String::from_utf8(raw) {
        Ok(text) => text,
        Err(e) => e.into_bytes().iter().map(|&b| b as char).collect(),
    }))
}

/// Sub-second resolution of a TOB1 record prefix, read from the declared name
/// of its second field (`SECONDS`, `NANOSECONDS`, ...).
///
/// TOB1 has no frame-time-resolution field — the record prefix carries the
/// unit in its own column name, and every TOB1 written by a CR-series logger
/// so far declares `NANOSECONDS`.
fn tob1_subsecond_res(second_field_name: &str) -> f64 {
    match second_field_name.trim().to_uppercase().as_str() {
        "NANOSECONDS" | "NSEC" => 1e-9,
        "MICROSECONDS" | "USEC" => 1e-6,
        "MILLISECONDS" | "MSEC" => 1e-3,
        _ => 1e-9,
    }
}

pub fn parse_tob_header(buff: &mut impl BufRead) -> std::io::Result<TobHeader> {
    // TOB1 has a 5-line prolog (environment, names, units, processing, types).
    // TOB2 and TOB3 insert a table/frame-geometry line after the environment
    // line, making 6.  Reading 6 lines from a TOB1 swallows the first binary
    // records as if they were a header line and leaves every following record
    // misaligned, so the count has to follow the format.
    let mut lines: Vec<String> = Vec::new();
    let mut is_tob1 = false;
    let mut is_tob2 = false;
    let mut header_lines = 6;
    let mut i = 0;
    while i < header_lines {
        let Some(line) = read_header_line(buff)? else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Incomplete header",
            ));
        };
        if i == 0 {
            if line.starts_with("\"TOB1\"") {
                is_tob1 = true;
                header_lines = 5;
            } else if line.starts_with("\"TOB2\"") {
                is_tob2 = true;
            } else if !line.starts_with("\"TOB3\"") {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Not a TOB1, TOB2 or TOB3 format",
                ));
            }
        }
        lines.push(line.trim().to_string());
        i += 1;
    }

    let l1 = split_csv_line(&lines[0]);
    // TOB1: no geometry line, and names/units/processing/types shift up one.
    let l2 = if is_tob1 {
        Vec::new()
    } else {
        split_csv_line(&lines[1])
    };
    let first_field_line = if is_tob1 { 1 } else { 2 };
    let csci_strings = split_csv_line(&lines[first_field_line + 3]);

    let frame_nbytes: usize = l2.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
    let rec_intvl_str = l2.get(1).map(|s| s.as_str()).unwrap_or("");

    let parts: Vec<&str> = rec_intvl_str.split_whitespace().collect();
    let mut rec_intvl = 0.0;
    if parts.len() == 2 {
        let val: f64 = parts[0].parse().unwrap_or(0.0);
        let multiplier = match parts[1].to_uppercase().as_str() {
            "HOUR" | "HOURS" => 3600.0,
            "MIN" | "MINS" => 60.0,
            "SEC" | "SECS" => 1.0,
            "MSEC" | "MSECS" => 1e-3,
            "USEC" | "USECS" => 1e-6,
            "NSEC" | "NSECS" => 1e-9,
            _ => 1.0,
        };
        rec_intvl = val * multiplier;
    }

    let ftr_str = l2.get(5).map(|s| s.as_str()).unwrap_or("");
    let frame_time_res = if is_tob1 {
        let field_names = split_csv_line(&lines[first_field_line]);
        tob1_subsecond_res(field_names.get(1).map(String::as_str).unwrap_or(""))
    } else {
        match ftr_str {
            "SecMsec" => 1e-3,
            "Sec100Usec" => 100e-6,
            "Sec10Usec" => 10e-6,
            "SecUsec" => 1e-6,
            "SecNanosec" => 1e-9,
            _ => 1e-3,
        }
    };

    let val_stamp: u16 = l2.get(4).and_then(|s| s.trim().parse().ok()).unwrap_or(0);
    let comp_val_stamp: u16 = 0xFFFF ^ val_stamp;

    let logger_model = l1.get(2).cloned().unwrap_or_default();
    let fp2_nan = fp2_nan_threshold(&logger_model);

    let mut names = split_csv_line(&lines[first_field_line]);
    let mut units = split_csv_line(&lines[first_field_line + 1]);
    let mut processing = split_csv_line(&lines[first_field_line + 2]);

    let mut csci_dtypes = Vec::new();
    let mut line_nbytes = 0;
    for c in &csci_strings {
        if let Some(dt) = CsciType::from_str(c) {
            line_nbytes += dt.size();
            csci_dtypes.push(dt);
        } else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Unknown type: {}", c),
            ));
        }
    }

    if is_tob1 {
        // TOB1 header lines 3-6 include SECONDS/NANOSECONDS/RECORD as first three fields.
        // Strip them — the reading loop extracts them from the raw record prefix bytes.
        if names.len() >= 3 {
            names.drain(..3);
        }
        if units.len() >= 3 {
            units.drain(..3);
        }
        if processing.len() >= 3 {
            processing.drain(..3);
        }
        if csci_dtypes.len() >= 3 {
            csci_dtypes.drain(..3);
        }
        line_nbytes = csci_dtypes.iter().map(|dt| dt.size()).sum();
    }

    let header_size = if is_tob2 { 8 } else { 12 };
    let data_segment = frame_nbytes.saturating_sub(header_size + 4);
    let data_nlines = if line_nbytes > 0 {
        data_segment / line_nbytes
    } else {
        0
    };
    let data_line_padding = if data_nlines > 0 {
        (data_segment - data_nlines * line_nbytes) / data_nlines
    } else {
        0
    };

    Ok(TobHeader {
        is_tob1,
        is_tob2,
        station_name: l1.get(1).cloned().unwrap_or_default(),
        logger_model,
        logger_sn: l1.get(3).cloned().unwrap_or_default(),
        logger_os: l1.get(4).cloned().unwrap_or_default(),
        logger_program: l1.get(5).cloned().unwrap_or_default(),
        logger_program_signature: l1.get(6).cloned().unwrap_or_default(),
        table_name: if is_tob1 {
            l1.get(7).cloned().unwrap_or_default()
        } else {
            l2.first().cloned().unwrap_or_default()
        },
        rec_intvl,
        frame_nbytes,
        frame_time_res,
        names,
        units,
        processing,
        csci_dtypes,
        data_nlines,
        line_nbytes,
        data_line_padding,
        val_stamp,
        comp_val_stamp,
        fp2_nan,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn minimal_header_lines(model: &str) -> String {
        format!(
            r#""TOB3","ST","{}","123","OS","PROG","SIG","2020-01-01 00:00:00"
"T1","1 SEC","64","0","4660","SecMsec","0","0","0"
"A","B"
"V","V"
"Smp","Smp"
"FP2","FP2"
"#,
            model
        )
    }

    #[test]
    fn fp2_nan_cr1000_and_cr1000x_use_7999() {
        assert_eq!(fp2_nan_threshold("CR1000"), 7999.0);
        assert_eq!(fp2_nan_threshold("CR1000X"), 7999.0);
        assert_eq!(fp2_nan_threshold("CR1000X.Std.01"), 7999.0);
    }

    #[test]
    fn fp2_nan_cr10_family_uses_6999_not_cr1000() {
        assert_eq!(fp2_nan_threshold("CR10X"), 6999.0);
        assert_eq!(fp2_nan_threshold("CR10"), 6999.0);
    }

    #[test]
    fn fp2_nan_unknown_defaults_7999() {
        assert_eq!(fp2_nan_threshold("CR6"), 7999.0);
    }

    #[test]
    fn parse_header_cr1000x_sets_model_and_fp2() {
        let data = minimal_header_lines("CR1000X");
        let mut c = Cursor::new(data);
        let h = parse_tob_header(&mut c).unwrap();
        assert!(h.logger_model.starts_with("CR1000"));
        assert_eq!(h.fp2_nan, 7999.0);
        assert_eq!(h.frame_nbytes, 64);
        assert_eq!(h.val_stamp, 4660);
        assert_eq!(h.line_nbytes, 4);
        assert_eq!(h.csci_dtypes.len(), 2);
    }

    #[test]
    fn parse_header_cr10x_sets_6999() {
        let data = minimal_header_lines("CR10X");
        let mut c = Cursor::new(data);
        let h = parse_tob_header(&mut c).unwrap();
        assert_eq!(h.fp2_nan, 6999.0);
    }

    #[test]
    fn parse_tob1_five_line_header_and_table_name() {
        let data = "\"TOB1\",\"ST\",\"CR1000\",\"7647\",\"OS\",\"PROG\",\"SIG\",\"Table1\"\n\
                    \"SECONDS\",\"NANOSECONDS\",\"RECORD\",\"A\"\n\
                    \"SECONDS\",\"NANOSECONDS\",\"RN\",\"V\"\n\
                    \"\",\"\",\"\",\"Smp\"\n\
                    \"ULONG\",\"ULONG\",\"ULONG\",\"FP2\"\n\
                    BINARYDATA";
        let mut c = Cursor::new(data);
        let h = parse_tob_header(&mut c).unwrap();
        assert!(h.is_tob1);
        // The table name is on the environment line, not on a geometry line.
        assert_eq!(h.table_name, "Table1");
        // SECONDS/NANOSECONDS/RECORD are stripped: one real column, 2 bytes.
        assert_eq!(h.names, vec!["A"]);
        assert_eq!(h.line_nbytes, 2);
        // NANOSECONDS means the record prefix counts nanoseconds.
        assert_eq!(h.frame_time_res, 1e-9);
    }

    #[test]
    fn parse_header_accepts_latin1_units() {
        // Campbell writes "W/m²" as a single 0xB2 byte, which is not UTF-8.
        let mut data = minimal_header_lines("CR1000").into_bytes();
        let pos = data
            .windows(3)
            .position(|w| w == b"\"V\"")
            .expect("unit line");
        data.splice(pos..pos + 3, b"\"W/m\xB2\"".iter().copied());
        let h = parse_tob_header(&mut Cursor::new(data)).unwrap();
        assert_eq!(h.units[0], "W/m\u{b2}");
    }

    #[test]
    fn parse_rejects_non_tob() {
        let mut c = Cursor::new("CSV,1,2\n");
        assert!(parse_tob_header(&mut c).is_err());
    }
}
