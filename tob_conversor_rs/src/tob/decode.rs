//! Binary field decoding → TOA5 field strings and typed values.

use chrono::DateTime;

use super::types::CsciType;

/// Seconds between Unix epoch (1970-01-01) and Campbell Scientific epoch (1990-01-01).
pub const TO_EPOCH: i64 = 631_152_000;

pub(crate) fn decode_fp2(bytes: &[u8], fp2_nan: f32) -> f32 {
    let fp2 = i16::from_be_bytes(bytes.try_into().unwrap());
    let sign = (fp2 >> 15) & 1;
    let exponent = (fp2 >> 13) & 3;
    let mantissa = (fp2 & 0x1FFF) as f32;
    let res = match exponent {
        0 => mantissa,
        1 => mantissa * 1e-1,
        2 => mantissa * 1e-2,
        _ => mantissa * 1e-3,
    };
    if res >= fp2_nan || res.is_nan() {
        return f32::NAN;
    }
    if sign == 1 { -res } else { res }
}

pub(crate) fn decode_fp4(bytes: &[u8]) -> f64 {
    let fp4 = u32::from_le_bytes(bytes.try_into().unwrap());
    let sign = (fp4 >> 31) & 1;
    let exponent = (fp4 >> 24) & 0x7F;
    let mantissa = (fp4 & 0x00FFFFFF) as f64;
    let mut res = (mantissa / 16777216.0) * f64::powi(2.0, (exponent as i32) - 64);
    if sign == 1 {
        res = -res;
    }
    if res.abs() >= 99999.0 {
        return f64::NAN;
    }
    res
}

pub fn format_ns_timestamp(ns: i64) -> String {
    let secs = ns / 1_000_000_000;
    let nsecs = (ns % 1_000_000_000) as u32;
    if let Some(dt_utc) = DateTime::from_timestamp(secs, nsecs) {
        let s = dt_utc.format("%Y-%m-%d %H:%M:%S%.3f").to_string();
        if s.ends_with(".000") {
            return format!("\"{}\"", &s[..s.len() - 4]);
        }
        return format!("\"{}\"", s);
    }
    "\"NAN\"".to_string()
}

pub(crate) fn file_datetime_stamp(ns: i64) -> String {
    // internal: output file naming only
    let secs = ns / 1_000_000_000;
    if let Some(dt_utc) = DateTime::from_timestamp(secs, 0) {
        return dt_utc.format("%Y_%m_%d_%H%M").to_string();
    }
    "unknown".to_string()
}

pub fn time_ns_from_frame_header(seconds: u32, subseconds: u32, frame_time_res: f64) -> i64 {
    ((seconds as i64) + TO_EPOCH) * 1_000_000_000
        + ((subseconds as f64) * frame_time_res * 1_000_000_000.0) as i64
}

/// One table field → one TOA5 cell string (strings and NaN/INF are quoted; numbers are bare).
pub fn format_field_toa5(csci_type: &CsciType, bytes: &[u8], fp2_nan: f32) -> String {
    match csci_type {
        CsciType::Ieee4 => {
            let val = f32::from_le_bytes(bytes.try_into().unwrap());
            if val.is_nan() {
                "\"NAN\"".to_string()
            } else {
                val.to_string()
            }
        }
        CsciType::Ieee4b => {
            let val = f32::from_be_bytes(bytes.try_into().unwrap());
            if val.is_nan() {
                "\"NAN\"".to_string()
            } else {
                val.to_string()
            }
        }
        CsciType::Ieee8 => {
            let val = f64::from_le_bytes(bytes.try_into().unwrap());
            if val.is_nan() {
                "\"NAN\"".to_string()
            } else {
                val.to_string()
            }
        }
        CsciType::Ieee8b => {
            let val = f64::from_be_bytes(bytes.try_into().unwrap());
            if val.is_nan() {
                "\"NAN\"".to_string()
            } else {
                val.to_string()
            }
        }
        CsciType::Fp2 => {
            let val = decode_fp2(bytes, fp2_nan);
            if val.is_nan() {
                "\"NAN\"".to_string()
            } else {
                val.to_string()
            }
        }
        CsciType::Fp4 => {
            let val = decode_fp4(bytes);
            if val.is_nan() {
                "\"NAN\"".to_string()
            } else {
                val.to_string()
            }
        }
        CsciType::Ushort => u16::from_le_bytes(bytes.try_into().unwrap()).to_string(),
        CsciType::Short => i16::from_le_bytes(bytes.try_into().unwrap()).to_string(),
        CsciType::Uint2 => u16::from_be_bytes(bytes.try_into().unwrap()).to_string(),
        CsciType::Int2 => i16::from_be_bytes(bytes.try_into().unwrap()).to_string(),
        CsciType::Uint4 => u32::from_be_bytes(bytes.try_into().unwrap()).to_string(),
        CsciType::Int4 => i32::from_be_bytes(bytes.try_into().unwrap()).to_string(),
        CsciType::Ulong => u32::from_le_bytes(bytes.try_into().unwrap()).to_string(),
        CsciType::Long => i32::from_le_bytes(bytes.try_into().unwrap()).to_string(),
        CsciType::Secnano => {
            let secs_raw = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
            let nanos = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
            let seconds = secs_raw as i64 + TO_EPOCH;
            let ns = seconds * 1_000_000_000 + nanos as i64;
            format_ns_timestamp(ns)
        }
        CsciType::Nsec => {
            let secs_raw = u32::from_be_bytes(bytes[0..4].try_into().unwrap());
            let nanos = u32::from_be_bytes(bytes[4..8].try_into().unwrap());
            let seconds = secs_raw as i64 + TO_EPOCH;
            let ns = seconds * 1_000_000_000 + nanos as i64;
            format_ns_timestamp(ns)
        }
        CsciType::Bool | CsciType::Bool8 => {
            if bytes[0] != 0 {
                "1".to_string()
            } else {
                "0".to_string()
            }
        }
        CsciType::Bool2 => {
            let v = u16::from_be_bytes(bytes.try_into().unwrap());
            if v != 0 {
                "1".to_string()
            } else {
                "0".to_string()
            }
        }
        CsciType::Bool4 => {
            let v = u32::from_be_bytes(bytes.try_into().unwrap());
            if v != 0 {
                "1".to_string()
            } else {
                "0".to_string()
            }
        }
        CsciType::Ascii(_) => {
            let s = String::from_utf8_lossy(bytes)
                .trim_end_matches('\0')
                .trim()
                .to_string();
            format!("\"{}\"", s)
        }
    }
}

/// Typed field value decoded from a TOB binary record. `Null` represents missing / NaN.
#[derive(Debug, Clone)]
pub enum FieldValue {
    F32(f32),
    F64(f64),
    I16(i16),
    U16(u16),
    I32(i32),
    U32(u32),
    /// Nanoseconds since Unix epoch (for Nsec / Secnano column types).
    TimestampNs(i64),
    Bool(bool),
    Str(String),
    Null,
}

/// Decode one table field into a typed [`FieldValue`]. NaN / invalid values become `Null`.
pub fn decode_field_value(csci_type: &CsciType, bytes: &[u8], fp2_nan: f32) -> FieldValue {
    match csci_type {
        CsciType::Ieee4 => {
            let v = f32::from_le_bytes(bytes.try_into().unwrap());
            if v.is_nan() {
                FieldValue::Null
            } else {
                FieldValue::F32(v)
            }
        }
        CsciType::Ieee4b => {
            let v = f32::from_be_bytes(bytes.try_into().unwrap());
            if v.is_nan() {
                FieldValue::Null
            } else {
                FieldValue::F32(v)
            }
        }
        CsciType::Ieee8 => {
            let v = f64::from_le_bytes(bytes.try_into().unwrap());
            if v.is_nan() {
                FieldValue::Null
            } else {
                FieldValue::F64(v)
            }
        }
        CsciType::Ieee8b => {
            let v = f64::from_be_bytes(bytes.try_into().unwrap());
            if v.is_nan() {
                FieldValue::Null
            } else {
                FieldValue::F64(v)
            }
        }
        CsciType::Fp2 => {
            let v = decode_fp2(bytes, fp2_nan);
            if v.is_nan() {
                FieldValue::Null
            } else {
                FieldValue::F32(v)
            }
        }
        CsciType::Fp4 => {
            let v = decode_fp4(bytes);
            if v.is_nan() {
                FieldValue::Null
            } else {
                FieldValue::F64(v)
            }
        }
        CsciType::Ushort => FieldValue::U16(u16::from_le_bytes(bytes.try_into().unwrap())),
        CsciType::Short => FieldValue::I16(i16::from_le_bytes(bytes.try_into().unwrap())),
        CsciType::Uint2 => FieldValue::U16(u16::from_be_bytes(bytes.try_into().unwrap())),
        CsciType::Int2 => FieldValue::I16(i16::from_be_bytes(bytes.try_into().unwrap())),
        CsciType::Uint4 => FieldValue::U32(u32::from_be_bytes(bytes.try_into().unwrap())),
        CsciType::Int4 => FieldValue::I32(i32::from_be_bytes(bytes.try_into().unwrap())),
        CsciType::Ulong => FieldValue::U32(u32::from_le_bytes(bytes.try_into().unwrap())),
        CsciType::Long => FieldValue::I32(i32::from_le_bytes(bytes.try_into().unwrap())),
        CsciType::Secnano => {
            let secs_raw = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
            let nanos = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
            let ns = (secs_raw as i64 + TO_EPOCH) * 1_000_000_000 + nanos as i64;
            FieldValue::TimestampNs(ns)
        }
        CsciType::Nsec => {
            let secs_raw = u32::from_be_bytes(bytes[0..4].try_into().unwrap());
            let nanos = u32::from_be_bytes(bytes[4..8].try_into().unwrap());
            let ns = (secs_raw as i64 + TO_EPOCH) * 1_000_000_000 + nanos as i64;
            FieldValue::TimestampNs(ns)
        }
        CsciType::Bool | CsciType::Bool8 => FieldValue::Bool(bytes[0] != 0),
        CsciType::Bool2 => FieldValue::Bool(u16::from_be_bytes(bytes.try_into().unwrap()) != 0),
        CsciType::Bool4 => FieldValue::Bool(u32::from_be_bytes(bytes.try_into().unwrap()) != 0),
        CsciType::Ascii(_) => {
            let s = String::from_utf8_lossy(bytes)
                .trim_end_matches('\0')
                .trim()
                .to_string();
            FieldValue::Str(s)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tob::types::CsciType::*;

    const FP2_CR1000: f32 = 7999.0;

    #[test]
    fn ieee4_le() {
        let b = 0.5f32.to_le_bytes();
        assert_eq!(format_field_toa5(&Ieee4, &b, FP2_CR1000), "0.5");
    }

    #[test]
    fn ieee4b_be() {
        let b = (-1.25f32).to_be_bytes();
        assert_eq!(format_field_toa5(&Ieee4b, &b, FP2_CR1000), "-1.25");
    }

    #[test]
    fn ieee8_le() {
        let b = std::f64::consts::PI.to_le_bytes();
        let s = format_field_toa5(&Ieee8, &b, FP2_CR1000);
        assert!(s.starts_with("3.14"));
    }

    #[test]
    fn ieee8b_be() {
        let b = 2.5f64.to_be_bytes();
        assert_eq!(format_field_toa5(&Ieee8b, &b, FP2_CR1000), "2.5");
    }

    #[test]
    fn fp2_battery_like() {
        // Same mantissa pattern as common station dumps (big-endian FP2)
        let b = [0x45u8, 0x8e];
        let s = format_field_toa5(&Fp2, &b, FP2_CR1000);
        assert!(s.parse::<f32>().unwrap() > 10.0 && s.parse::<f32>().unwrap() < 20.0);
    }

    #[test]
    fn fp2_nan_sentinel_cr1000() {
        // Exponent 0, mantissa ≥ 7999 → treated as missing (CR1000/CR1000X threshold)
        let v: i16 = 8000;
        let b = v.to_be_bytes();
        assert_eq!(format_field_toa5(&Fp2, &b, FP2_CR1000), "\"NAN\"");
    }

    #[test]
    fn fp4_zero() {
        let s = format_field_toa5(&Fp4, &[0u8; 4], FP2_CR1000);
        assert_eq!(s, "0");
    }

    #[test]
    fn ushort_short_uint2_int2() {
        assert_eq!(
            format_field_toa5(&Ushort, &1000u16.to_le_bytes(), FP2_CR1000),
            "1000"
        );
        assert_eq!(
            format_field_toa5(&Short, &(-5i16).to_le_bytes(), FP2_CR1000),
            "-5"
        );
        assert_eq!(
            format_field_toa5(&Uint2, &200u16.to_be_bytes(), FP2_CR1000),
            "200"
        );
        assert_eq!(
            format_field_toa5(&Int2, &(-9i16).to_be_bytes(), FP2_CR1000),
            "-9"
        );
    }

    #[test]
    fn uint4_int4_ulong_long() {
        assert_eq!(
            format_field_toa5(&Uint4, &1_234_567u32.to_be_bytes(), FP2_CR1000),
            "1234567"
        );
        assert_eq!(
            format_field_toa5(&Int4, &(-99i32).to_be_bytes(), FP2_CR1000),
            "-99"
        );
        assert_eq!(
            format_field_toa5(&Ulong, &42u32.to_le_bytes(), FP2_CR1000),
            "42"
        );
        assert_eq!(
            format_field_toa5(&Long, &(-1i32).to_le_bytes(), FP2_CR1000),
            "-1"
        );
    }

    #[test]
    fn secnano_nsec_timestamps() {
        let mut b = [0u8; 8];
        b[0..4].copy_from_slice(&100u32.to_le_bytes());
        b[4..8].copy_from_slice(&500_000u32.to_le_bytes());
        let s = format_field_toa5(&Secnano, &b, FP2_CR1000);
        assert!(s.starts_with('"'));
        assert_ne!(s, "\"NAN\"");

        let mut bb = [0u8; 8];
        bb[0..4].copy_from_slice(&200u32.to_be_bytes());
        bb[4..8].copy_from_slice(&0u32.to_be_bytes());
        let s2 = format_field_toa5(&Nsec, &bb, FP2_CR1000);
        assert!(s2.starts_with('"'));
        assert_ne!(s2, "\"NAN\"");
    }

    #[test]
    fn bool_variants() {
        assert_eq!(format_field_toa5(&Bool, &[0], FP2_CR1000), "0");
        assert_eq!(format_field_toa5(&Bool, &[1], FP2_CR1000), "1");
        assert_eq!(
            format_field_toa5(&Bool2, &1u16.to_be_bytes(), FP2_CR1000),
            "1"
        );
        assert_eq!(
            format_field_toa5(&Bool4, &0u32.to_be_bytes(), FP2_CR1000),
            "0"
        );
    }

    #[test]
    fn ascii_field() {
        let v = b"hello\0\0".to_vec();
        assert_eq!(format_field_toa5(&Ascii(8), &v, FP2_CR1000), "\"hello\"");
    }

    #[test]
    fn time_ns_roundtrip_order() {
        let ns = time_ns_from_frame_header(100, 0, 1e-3);
        assert!(ns > 0);
    }
}
