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

pub fn parse_tob_header(buff: &mut impl BufRead) -> std::io::Result<TobHeader> {
    let mut lines = Vec::new();
    let mut is_tob1 = false;
    let mut is_tob2 = false;
    for i in 0..6 {
        let mut line = String::new();
        let bytes = buff.read_line(&mut line)?;
        if bytes == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Incomplete header",
            ));
        }
        if i == 0 {
            if line.starts_with("\"TOB1\"") {
                is_tob1 = true;
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
    }

    let l1 = split_csv_line(&lines[0]);
    let l2 = split_csv_line(&lines[1]);
    let csci_strings = split_csv_line(&lines[5]);

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
    let frame_time_res = match ftr_str {
        "SecMsec" => 1e-3,
        "Sec100Usec" => 100e-6,
        "Sec10Usec" => 10e-6,
        "SecUsec" => 1e-6,
        "SecNanosec" => 1e-9,
        _ => 1e-3,
    };

    let val_stamp: u16 = l2.get(4).and_then(|s| s.trim().parse().ok()).unwrap_or(0);
    let comp_val_stamp: u16 = 0xFFFF ^ val_stamp;

    let logger_model = l1.get(2).cloned().unwrap_or_default();
    let fp2_nan = fp2_nan_threshold(&logger_model);

    let mut names = split_csv_line(&lines[2]);
    let mut units = split_csv_line(&lines[3]);
    let mut processing = split_csv_line(&lines[4]);

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
        table_name: l2.get(0).cloned().unwrap_or_default(),
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
    fn parse_rejects_non_tob() {
        let mut c = Cursor::new("CSV,1,2\n");
        assert!(parse_tob_header(&mut c).is_err());
    }
}
