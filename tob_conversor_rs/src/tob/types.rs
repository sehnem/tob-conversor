//! CSI table column types (TOB3 line layout).

use arrow_schema::{DataType, TimeUnit};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CsciType {
    Ieee4,
    Ieee4b,
    Ieee8,
    Ieee8b,
    Fp2,
    Fp4,
    Ushort,
    Short,
    Uint2,
    Int2,
    Uint4,
    Int4,
    Ulong,
    Long,
    Nsec,
    Secnano,
    Bool,
    Bool8,
    Bool4,
    Bool2,
    Ascii(usize),
}

impl CsciType {
    pub fn from_str(s: &str) -> Option<Self> {
        let s = s.trim_matches('"');
        if s.starts_with("ASCII(") {
            let n = s[6..s.len() - 1].parse().unwrap_or(0);
            return Some(CsciType::Ascii(n));
        }
        match s {
            "IEEE4" => Some(CsciType::Ieee4),
            "IEEE4B" => Some(CsciType::Ieee4b),
            "IEEE8" => Some(CsciType::Ieee8),
            "IEEE8B" => Some(CsciType::Ieee8b),
            "FP2" => Some(CsciType::Fp2),
            "FP4" => Some(CsciType::Fp4),
            "USHORT" => Some(CsciType::Ushort),
            "SHORT" => Some(CsciType::Short),
            "UINT2" => Some(CsciType::Uint2),
            "INT2" => Some(CsciType::Int2),
            "UINT4" => Some(CsciType::Uint4),
            "INT4" => Some(CsciType::Int4),
            "ULONG" => Some(CsciType::Ulong),
            "LONG" => Some(CsciType::Long),
            "NSEC" => Some(CsciType::Nsec),
            "SECNANO" => Some(CsciType::Secnano),
            "BOOL" => Some(CsciType::Bool),
            "BOOL8" => Some(CsciType::Bool8),
            "BOOL4" => Some(CsciType::Bool4),
            "BOOL2" => Some(CsciType::Bool2),
            _ => None,
        }
    }

    pub fn to_arrow_type(&self) -> DataType {
        match self {
            CsciType::Ieee4 | CsciType::Ieee4b | CsciType::Fp2 => DataType::Float32,
            CsciType::Ieee8 | CsciType::Ieee8b | CsciType::Fp4 => DataType::Float64,
            CsciType::Short | CsciType::Int2 => DataType::Int16,
            CsciType::Ushort | CsciType::Uint2 => DataType::UInt16,
            CsciType::Long | CsciType::Int4 => DataType::Int32,
            CsciType::Ulong | CsciType::Uint4 => DataType::UInt32,
            CsciType::Bool | CsciType::Bool8 | CsciType::Bool2 | CsciType::Bool4 => {
                DataType::Boolean
            }
            CsciType::Nsec | CsciType::Secnano => {
                DataType::Timestamp(TimeUnit::Nanosecond, Some("UTC".into()))
            }
            CsciType::Ascii(_) => DataType::Utf8,
        }
    }

    pub fn size(&self) -> usize {
        match self {
            CsciType::Bool | CsciType::Bool8 => 1,
            CsciType::Fp2
            | CsciType::Ushort
            | CsciType::Short
            | CsciType::Uint2
            | CsciType::Int2
            | CsciType::Bool2 => 2,
            CsciType::Ieee4
            | CsciType::Ieee4b
            | CsciType::Fp4
            | CsciType::Uint4
            | CsciType::Int4
            | CsciType::Ulong
            | CsciType::Long
            | CsciType::Bool4 => 4,
            CsciType::Ieee8 | CsciType::Ieee8b | CsciType::Nsec | CsciType::Secnano => 8,
            CsciType::Ascii(n) => *n,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CsciType::*;

    #[test]
    fn from_str_all_primitives() {
        let cases = [
            ("IEEE4", Ieee4),
            ("IEEE4B", Ieee4b),
            ("IEEE8", Ieee8),
            ("IEEE8B", Ieee8b),
            ("FP2", Fp2),
            ("FP4", Fp4),
            ("USHORT", Ushort),
            ("SHORT", Short),
            ("UINT2", Uint2),
            ("INT2", Int2),
            ("UINT4", Uint4),
            ("INT4", Int4),
            ("ULONG", Ulong),
            ("LONG", Long),
            ("NSEC", Nsec),
            ("SECNANO", Secnano),
            ("BOOL", Bool),
            ("BOOL8", Bool8),
            ("BOOL4", Bool4),
            ("BOOL2", Bool2),
        ];
        for (s, expected) in cases {
            assert_eq!(super::CsciType::from_str(s), Some(expected), "type {}", s);
        }
        assert_eq!(super::CsciType::from_str("\"FP2\""), Some(Fp2));
        assert_eq!(super::CsciType::from_str("ASCII(16)"), Some(Ascii(16)));
        assert_eq!(super::CsciType::from_str("\"ASCII(8)\""), Some(Ascii(8)));
    }

    #[test]
    fn sizes_sum_matches_line_layout() {
        let line: Vec<super::CsciType> = vec![Fp2, Ieee4b, Fp2];
        let sum: usize = line.iter().map(|t| t.size()).sum();
        assert_eq!(sum, 2 + 4 + 2);
    }
}
