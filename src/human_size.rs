#[derive(Debug, Clone, Copy)]
pub enum UnitSpec {
    Raw,
    K,
    Ki,
    M,
    Mi,
    G,
    Gi,
    T,
    Ti,
    Auto,
    AutoSi,
}

/// Split `num / den` into an integer part and one decimal digit, rounding
/// halves up.
///
/// The old form computed `(num * 10) / den` with integer division, which
/// truncates: 256 bytes is exactly 0.25 KiB, so the tenths digit came out as
/// 2 ("0.2 KiB") instead of rounding to 3 ("0.3 KiB"). It also overflowed for
/// any size above ~1.8 EB. Splitting the quotient from the remainder keeps
/// every intermediate far below `u64::MAX`, since `rem < den`.
fn round_half_up_scaled(num: u64, den: u64) -> (u64, u32) {
    let whole = num / den;
    let rem = num % den;
    // round-half-up of `10 * rem / den`, which can carry into `whole`.
    let tenths = (20 * rem + den) / (2 * den);
    if tenths >= 10 {
        (whole + 1, 0)
    } else {
        (whole, tenths as u32)
    }
}

/// Format using one fixed unit: divide, one decimal (round-half-up), suffix.
fn format_fixed(bytes: u64, divisor: u64, suffix: &str) -> String {
    let (int_part, frac) = round_half_up_scaled(bytes, divisor);
    format!("{}.{} {}", int_part, frac, suffix)
}

pub fn format_size(bytes: u64, unit: UnitSpec) -> String {
    match unit {
        UnitSpec::Raw => bytes.to_string(),
        UnitSpec::K => format_fixed(bytes, 1_000, "K"),
        UnitSpec::Ki => format_fixed(bytes, 1_024, "KiB"),
        UnitSpec::M => format_fixed(bytes, 1_000_000, "M"),
        UnitSpec::Mi => format_fixed(bytes, 1_048_576, "MiB"),
        UnitSpec::G => format_fixed(bytes, 1_000_000_000, "G"),
        UnitSpec::Gi => format_fixed(bytes, 1_073_741_824, "GiB"),
        UnitSpec::T => format_fixed(bytes, 1_000_000_000_000, "T"),
        UnitSpec::Ti => format_fixed(bytes, 1_099_511_627_776, "TiB"),
        UnitSpec::Auto => format_auto(bytes, true),
        UnitSpec::AutoSi => format_auto(bytes, false),
    }
}

fn auto_pick_unit(bytes: u64, is_iec: bool) -> (u64, u32, &'static str) {
    if bytes == 0 {
        return (0, 0, "B");
    }

    let base: u64 = if is_iec { 1024 } else { 1000 };
    let tiers = if is_iec {
        &[
            (1_099_511_627_776u64, "TiB"),
            (1_073_741_824u64, "GiB"),
            (1_048_576u64, "MiB"),
            (1_024u64, "KiB"),
        ]
    } else {
        &[
            (1_000_000_000_000u64, "T"),
            (1_000_000_000u64, "G"),
            (1_000_000u64, "M"),
            (1_000u64, "K"),
        ]
    };

    for (idx, &(divisor, suffix)) in tiers.iter().enumerate() {
        if bytes >= divisor {
            let (int_part, frac) = round_half_up_scaled(bytes, divisor);
            // Rounding can carry into the next tier (1023.95 KiB would print as
            // "1024.0 KiB"); promote instead so the unit is always the largest
            // that fits.
            if int_part >= base && idx > 0 {
                let (next_divisor, next_suffix) = tiers[idx - 1];
                let (ip, fr) = round_half_up_scaled(bytes, next_divisor);
                return (ip, fr, next_suffix);
            }
            return (int_part, frac, suffix);
        }
    }

    (bytes, 0, "B")
}

fn format_auto(bytes: u64, is_iec: bool) -> String {
    let (int_part, frac, suffix) = auto_pick_unit(bytes, is_iec);
    if (suffix == "B" || !is_iec) && int_part == bytes {
        return format!("{} B", bytes);
    }
    format!("{}.{} {}", int_part, frac, suffix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rounds_halves_up_at_the_tenths_digit() {
        // 256/1024 is exactly 0.25 KiB: the tenths digit is exactly 2.5, so
        // it rounds up. Truncating division reported "0.2 KiB".
        assert_eq!(format_size(256, UnitSpec::Ki), "0.3 KiB");
        assert_eq!(format_size(1536, UnitSpec::Ki), "1.5 KiB");
        assert_eq!(format_size(1_536_000, UnitSpec::M), "1.5 M");
        // 9950/1000 is 9.95: the tenth rounds up to 10 and carries.
        assert_eq!(format_size(9_950, UnitSpec::K), "10.0 K");
    }

    #[test]
    fn test_half_up_rounding_does_not_overflow_on_huge_sizes() {
        // The old `(num * 10) / den` overflowed a u64 debug build here.
        let num = u64::MAX / 2;
        let (int_part, frac) = round_half_up_scaled(num, 1024);
        let expected_tenths = ((num as u128) * 20 + 1024) / (2 * 1024);
        assert_eq!(int_part as u128 * 10 + frac as u128, expected_tenths);
        assert!(frac <= 9);
    }

    #[test]
    fn test_raw_zero() {
        assert_eq!(format_size(0, UnitSpec::Raw), "0");
    }

    #[test]
    fn test_kilobytes_1k_raw() {
        assert_eq!(format_size(1_000, UnitSpec::K), "1.0 K");
    }

    #[test]
    fn test_kibibytes_1536() {
        assert_eq!(format_size(1_536, UnitSpec::Ki), "1.5 KiB");
    }

    #[test]
    fn test_megabytes_1536000_raw() {
        assert_eq!(format_size(1_536_000, UnitSpec::M), "1.5 M");
    }

    #[test]
    fn test_mebibytes_rounding_up() {
        assert_eq!(format_size(2_097_152, UnitSpec::Mi), "2.0 MiB");
    }

    #[test]
    fn test_auto_promotes_across_a_unit_boundary() {
        // 1048575 bytes is 1023.999 KiB, which rounds to 1024.0 KiB; the unit
        // must promote to MiB rather than print "1024.0 KiB".
        assert_eq!(format_size(1_048_575, UnitSpec::Auto), "1.0 MiB");
        // Same for SI: 999999 bytes rounds up to 1000.0 K -> 1.0 M.
        assert_eq!(format_size(999_999, UnitSpec::AutoSi), "1.0 M");
    }

    #[test]
    fn test_auto_one_byte() {
        assert_eq!(format_size(1, UnitSpec::Auto), "1 B");
    }

    #[test]
    fn test_auto_si_1500_bytes() {
        assert_eq!(format_size(1_500, UnitSpec::AutoSi), "1.5 K");
    }

    #[test]
    fn test_auto_iec_1536_bytes() {
        assert_eq!(format_size(1_536, UnitSpec::Auto), "1.5 KiB");
    }

    #[test]
    fn test_auto_si_1500000_bytes() {
        assert_eq!(format_size(1_500_000, UnitSpec::AutoSi), "1.5 M");
    }

    #[test]
    fn test_auto_iec_1433600_bytes_is_1_4_mib() {
        // 1433600 / 1048576 = 1.3672 -> 1.4 MiB.
        assert_eq!(format_size(1_433_600, UnitSpec::Auto), "1.4 MiB");
    }

    #[test]
    fn test_auto_si_gigabytes() {
        assert_eq!(format_size(2_500_000_000, UnitSpec::AutoSi), "2.5 G");
    }

    #[test]
    fn test_auto_zero_bytes() {
        assert_eq!(format_size(0, UnitSpec::Auto), "0 B");
    }
}
