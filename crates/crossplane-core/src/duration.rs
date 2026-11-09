//! Go `time.ParseDuration`, so flags and the `crossplane.io/poll-interval` annotation parse the
//! same strings as the Go provider.

use std::time::Duration;

const MAX_NANOS: u128 = i64::MAX as u128;

/// A negative value is an error here. The annotation reader treats any error as "ignore".
pub fn parse(s: &str) -> Result<Duration, String> {
    let orig = s;
    let invalid = || format!("time: invalid duration {orig:?}");
    let (negative, mut rest) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    if rest == "0" {
        return Ok(Duration::ZERO);
    }
    if rest.is_empty() {
        return Err(invalid());
    }

    let mut total: u128 = 0;
    while !rest.is_empty() {
        if !rest.starts_with(|c: char| c == '.' || c.is_ascii_digit()) {
            return Err(invalid());
        }
        let (int_digits, after_int) = split_digits(rest);
        rest = after_int;
        let mut frac_digits = "";
        if let Some(after_dot) = rest.strip_prefix('.') {
            let (frac, after_frac) = split_digits(after_dot);
            frac_digits = frac;
            rest = after_frac;
        }
        if int_digits.is_empty() && frac_digits.is_empty() {
            return Err(invalid());
        }

        let unit_len = rest
            .find(|c: char| c == '.' || c.is_ascii_digit())
            .unwrap_or(rest.len());
        if unit_len == 0 {
            return Err("time: missing unit in duration ".to_owned() + &format!("{orig:?}"));
        }
        let (unit, after_unit) = rest.split_at(unit_len);
        rest = after_unit;
        let scale: u128 = match unit {
            "ns" => 1,
            "us" | "\u{b5}s" | "\u{3bc}s" => 1_000,
            "ms" => 1_000_000,
            "s" => 1_000_000_000,
            "m" => 60_000_000_000,
            "h" => 3_600_000_000_000,
            _ => return Err(format!("time: unknown unit {unit:?} in duration {orig:?}")),
        };

        let whole: u128 = if int_digits.is_empty() {
            0
        } else {
            int_digits.parse().map_err(|_| invalid())?
        };
        let mut nanos = whole
            .checked_mul(scale)
            .filter(|n| *n <= MAX_NANOS)
            .ok_or_else(invalid)?;
        let mut frac_scale = scale;
        for d in frac_digits.bytes() {
            frac_scale /= 10;
            if frac_scale == 0 {
                break;
            }
            nanos += u128::from(d - b'0') * frac_scale;
        }
        total += nanos;
        if total > MAX_NANOS {
            return Err(invalid());
        }
    }

    if negative {
        return Err(format!("negative duration {orig:?}"));
    }
    Ok(Duration::from_nanos(total as u64))
}

fn split_digits(s: &str) -> (&str, &str) {
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    s.split_at(end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_parse_duration_examples() {
        let ok = [
            ("0", Duration::ZERO),
            ("+5s", Duration::from_secs(5)),
            ("1h", Duration::from_secs(3600)),
            ("1m", Duration::from_secs(60)),
            ("5s", Duration::from_secs(5)),
            ("1h30m", Duration::from_secs(5400)),
            ("1.5s", Duration::from_millis(1500)),
            (".5s", Duration::from_millis(500)),
            ("5.s", Duration::from_secs(5)),
            ("100ms", Duration::from_millis(100)),
            ("3\u{b5}s", Duration::from_micros(3)),
            ("3us", Duration::from_micros(3)),
            ("7ns", Duration::from_nanos(7)),
            ("0.000000001s", Duration::from_nanos(1)),
            ("0s", Duration::ZERO),
        ];
        for (s, want) in ok {
            assert_eq!(Ok(want), parse(s), "parse({s:?})");
        }
        for s in [
            "",
            "-",
            "5",
            "s",
            ".s",
            "1x",
            "1h-",
            "-5s",
            "1 s",
            "9223372036854775808ns",
            "10000000h",
        ] {
            assert!(parse(s).is_err(), "parse({s:?}) should fail");
        }
        assert_eq!(Err("time: missing unit in duration \"5\"".to_owned()), parse("5"));
        assert_eq!(
            Err("time: unknown unit \"x\" in duration \"1x\"".to_owned()),
            parse("1x")
        );
    }
}
