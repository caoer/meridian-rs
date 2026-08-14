//! `DuckDB`-text rendering for sql result cells (card sql-scalar-type-render,
//! dogfood r6 § S1): the scalar families that `DuckDB`'s Arrow face hands over
//! as raw payloads — timestamp, date, time, interval, decimal, and the nested
//! containers — render here as the SQL text `DuckDB` itself would print,
//! never a Rust `Debug` repr.
//!
//! Ground truth is pinned two ways: probed against raw `DuckDB` CLI v1.5.4
//! (the r6 round's second instrument), and read from the engine's own source —
//! `IntervalToStringCast`/`DateToStringCast` (`cast_helpers.hpp`) for the
//! scalar grammars, `NestedToVarcharCast::LOOKUP_TABLE` +
//! `CalculateEscapedStringLength` (`vector_cast_helpers.hpp`) for the
//! nested-context quoting rule.
//!
//! Two deliberate divergences, both in corners no corpus projection reaches:
//! floats inside nested containers use Rust's shortest round-trip form, which
//! differs from `DuckDB`'s formatter in exponent style (`1e40` vs `1e+40`,
//! threshold near 1e15); and a timestamptz NESTED in a container renders
//! without its `+00` marker because the owned-value lane loses the column's
//! timezone flag (top-level cells keep it).

use duckdb::types::{TimeUnit, Value};

/// One timestamp cell. `tz` marks a `TIMESTAMP WITH TIME ZONE` column: the
/// engine runs without ICU, so the value is UTC and `DuckDB` would print the
/// `+00` marker.
pub(crate) fn timestamp_text(unit: TimeUnit, v: i64, tz: bool) -> String {
    // DuckDB's timestamp_t sentinels (shared by every timestamp width).
    if v == i64::MAX {
        return "infinity".to_owned();
    }
    if v == -i64::MAX {
        return "-infinity".to_owned();
    }
    let (scale, frac_width) = match unit {
        TimeUnit::Second => (1, 0),
        TimeUnit::Millisecond => (1_000, 3),
        TimeUnit::Microsecond => (1_000_000, 6),
        TimeUnit::Nanosecond => (1_000_000_000, 9),
    };
    let day = 86_400 * scale;
    let days = v.div_euclid(day);
    let rem = v.rem_euclid(day);
    let mut s = ymd_text(days);
    s.push(' ');
    push_hms(&mut s, rem / scale);
    push_frac(&mut s, rem % scale, frac_width);
    if tz {
        s.push_str("+00");
    }
    s
}

/// One date cell: days since the Unix epoch.
pub(crate) fn date_text(days: i32) -> String {
    // date_t::infinity / ::ninfinity.
    if days == i32::MAX {
        return "infinity".to_owned();
    }
    if days == -i32::MAX {
        return "-infinity".to_owned();
    }
    ymd_text(i64::from(days))
}

/// One time cell. `DuckDB` times are microseconds; the other widths exist
/// only because [`TimeUnit`] carries them.
pub(crate) fn time_text(unit: TimeUnit, v: i64) -> String {
    let (scale, frac_width) = match unit {
        TimeUnit::Second => (1, 0),
        TimeUnit::Millisecond => (1_000, 3),
        TimeUnit::Microsecond => (1_000_000, 6),
        TimeUnit::Nanosecond => (1_000_000_000, 9),
    };
    let mut s = String::new();
    push_hms(&mut s, v.div_euclid(scale));
    push_frac(&mut s, v.rem_euclid(scale), frac_width);
    s
}

/// One interval cell — `IntervalToStringCast::Format`: nonzero year/month/day
/// components with singular/plural names, a time part when sub-day micros
/// remain, `00:00:00` when everything is zero.
pub(crate) fn interval_text(months: i32, days: i32, nanos: i64) -> String {
    let mut s = String::new();
    if months != 0 {
        let years = months / 12;
        push_component(&mut s, i64::from(years), "year");
        push_component(&mut s, i64::from(months - years * 12), "month");
    }
    push_component(&mut s, i64::from(days), "day");
    // The wire carries nanos; DuckDB's own resolution is micros.
    let micros = nanos / 1_000;
    if micros != 0 {
        if !s.is_empty() {
            s.push(' ');
        }
        if micros < 0 {
            s.push('-');
        }
        let m = micros.unsigned_abs();
        let (hour, m) = (m / 3_600_000_000, m % 3_600_000_000);
        let (min, m) = (m / 60_000_000, m % 60_000_000);
        let (sec, sub) = (m / 1_000_000, m % 1_000_000);
        if hour < 10 {
            s.push('0');
        }
        let _ = std::fmt::Write::write_fmt(&mut s, format_args!("{hour}:{min:02}:{sec:02}"));
        #[allow(clippy::cast_possible_wrap)] // sub < 1e6
        push_frac(&mut s, sub as i64, 6);
    } else if s.is_empty() {
        s.push_str("00:00:00");
    }
    s
}

/// One owned value in `DuckDB`'s NESTED context — the form used inside
/// lists, structs, and maps, where text-like values quote by content.
pub(crate) fn nested_text(v: &Value) -> String {
    match v {
        Value::Text(s) | Value::Enum(s) => quote_if_needed(s),
        Value::Blob(b) => quote_if_needed(&blob_text(b)),
        Value::Date32(d) => quote_if_needed(&date_text(*d)),
        Value::Time64(u, t) => quote_if_needed(&time_text(*u, *t)),
        Value::Timestamp(u, t) => quote_if_needed(&timestamp_text(*u, *t, false)),
        Value::Interval {
            months,
            days,
            nanos,
        } => quote_if_needed(&interval_text(*months, *days, *nanos)),
        Value::Decimal(d) => d.to_string(),
        _ => raw_text(v),
    }
}

/// One owned value in the RAW context — top-level cells and union members,
/// where `DuckDB` writes the member text with no quoting at all. Containers
/// render identically in both contexts.
pub(crate) fn raw_text(v: &Value) -> String {
    match v {
        Value::Null => "NULL".to_owned(),
        Value::Boolean(b) => b.to_string(),
        Value::TinyInt(n) => n.to_string(),
        Value::SmallInt(n) => n.to_string(),
        Value::Int(n) => n.to_string(),
        Value::BigInt(n) => n.to_string(),
        Value::HugeInt(n) => n.to_string(),
        Value::UTinyInt(n) => n.to_string(),
        Value::USmallInt(n) => n.to_string(),
        Value::UInt(n) => n.to_string(),
        Value::UBigInt(n) => n.to_string(),
        Value::Float(f) => float32_text(*f),
        Value::Double(f) => float64_text(*f),
        Value::Decimal(d) => d.to_string(),
        Value::Text(s) | Value::Enum(s) => s.clone(),
        Value::Blob(b) => blob_text(b),
        Value::Date32(d) => date_text(*d),
        Value::Time64(u, t) => time_text(*u, *t),
        Value::Timestamp(u, t) => timestamp_text(*u, *t, false),
        Value::Interval {
            months,
            days,
            nanos,
        } => interval_text(*months, *days, *nanos),
        Value::List(items) | Value::Array(items) => {
            let parts: Vec<String> = items.iter().map(nested_text).collect();
            format!("[{}]", parts.join(", "))
        }
        Value::Struct(fields) => {
            let parts: Vec<String> = fields
                .iter()
                .map(|(k, v)| format!("{}: {}", quote_always(k), nested_text(v)))
                .collect();
            format!("{{{}}}", parts.join(", "))
        }
        Value::Map(pairs) => {
            let parts: Vec<String> = pairs
                .iter()
                .map(|(k, v)| format!("{}={}", nested_text(k), nested_text(v)))
                .collect();
            format!("{{{}}}", parts.join(", "))
        }
        Value::Union(member) => raw_text(member),
    }
}

/// `DuckDB`'s float text for the non-finite values; finite ones keep Rust's
/// shortest round-trip form (see the module doc's divergence note).
fn float32_text(f: f32) -> String {
    if f.is_nan() {
        "nan".to_owned()
    } else if f.is_infinite() {
        if f < 0.0 { "-inf" } else { "inf" }.to_owned()
    } else {
        format!("{f:?}")
    }
}

fn float64_text(f: f64) -> String {
    if f.is_nan() {
        "nan".to_owned()
    } else if f.is_infinite() {
        if f < 0.0 { "-inf" } else { "inf" }.to_owned()
    } else {
        format!("{f:?}")
    }
}

/// `Blob::ToString`: printable ASCII bytes raw (backslash and both quote
/// bytes excepted), the rest as `\xHH` uppercase hex.
fn blob_text(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len());
    for &b in bytes {
        if b.is_ascii_graphic() && !matches!(b, b'\\' | b'\'' | b'"') || b == b' ' {
            s.push(char::from(b));
        } else {
            let _ = write!(s, "\\x{b:02X}");
        }
    }
    s
}

/// The nested-context quoting rule (`CalculateEscapedStringLength`, non-key
/// arm): quote when the text is empty, opens or closes with whitespace, is
/// `NULL` in any case, or carries a structural byte; escape `\` and `'`.
fn quote_if_needed(s: &str) -> String {
    let needs = s.is_empty()
        || s.bytes().next().is_some_and(is_space)
        || (s.len() >= 2 && s.bytes().next_back().is_some_and(is_space))
        || s.eq_ignore_ascii_case("null")
        || s.bytes().any(|b| {
            matches!(
                b,
                b'"' | b'\'' | b'(' | b')' | b',' | b':' | b'=' | b'[' | b']' | b'{' | b'}'
            )
        });
    if needs { quote_always(s) } else { s.to_owned() }
}

/// The always-quoted form (struct keys, and any text `quote_if_needed`
/// deemed unservable bare).
fn quote_always(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('\'');
    out
}

/// `StringUtil::CharacterIsSpace`.
fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r')
}

/// Proleptic-Gregorian date text from days since the epoch, `DuckDB`'s
/// `DateToStringCast` shape: 4-digit-min year, ` (BC)` for years <= 0.
fn ymd_text(days: i64) -> String {
    let (y, m, d) = civil_from_days(days);
    if y <= 0 {
        format!("{:04}-{m:02}-{d:02} (BC)", 1 - y)
    } else {
        format!("{y:04}-{m:02}-{d:02}")
    }
}

/// Howard Hinnant's `civil_from_days`: days since 1970-01-01 to (year,
/// month, day), exact over the whole proleptic range.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // m in 1..=12, d in 1..=31
    (y + i64::from(m <= 2), m as u32, d as u32)
}

/// `HH:MM:SS` from seconds-of-day (hours widen past two digits untouched,
/// as `DuckDB` prints them).
fn push_hms(s: &mut String, secs_of_day: i64) {
    let (h, rem) = (secs_of_day / 3_600, secs_of_day % 3_600);
    let _ = std::fmt::Write::write_fmt(s, format_args!("{h:02}:{:02}:{:02}", rem / 60, rem % 60));
}

/// The fractional-seconds tail at `width` digits, trailing zeros trimmed,
/// omitted entirely at zero.
fn push_frac(s: &mut String, sub: i64, width: usize) {
    if sub == 0 || width == 0 {
        return;
    }
    let digits = format!("{sub:0width$}");
    s.push('.');
    s.push_str(digits.trim_end_matches('0'));
}

/// One interval component (`FormatIntervalValue`): skipped at zero,
/// space-separated, pluralized except at +/-1.
fn push_component(s: &mut String, value: i64, name: &str) {
    if value == 0 {
        return;
    }
    if !s.is_empty() {
        s.push(' ');
    }
    s.push_str(&value.to_string());
    s.push(' ');
    s.push_str(name);
    if value != 1 && value != -1 {
        s.push('s');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Every expected string below is pinned against raw DuckDB CLI v1.5.4
    // (worktree .scratch/s1-*.duckdb-1.5.4.txt receipts).

    #[test]
    fn timestamps_render_at_their_width_with_trimmed_fractions() {
        let us = TimeUnit::Microsecond;
        assert_eq!(
            timestamp_text(us, 1_786_708_800_000_000, false),
            "2026-08-14 12:00:00"
        );
        assert_eq!(
            timestamp_text(us, 1_786_708_800_120_000, false),
            "2026-08-14 12:00:00.12"
        );
        assert_eq!(
            timestamp_text(us, 1_786_708_800_000_000, true),
            "2026-08-14 12:00:00+00"
        );
        assert_eq!(
            timestamp_text(TimeUnit::Second, 1_786_708_800, false),
            "2026-08-14 12:00:00"
        );
        assert_eq!(
            timestamp_text(TimeUnit::Millisecond, 1_786_708_800_123, false),
            "2026-08-14 12:00:00.123"
        );
        assert_eq!(
            timestamp_text(TimeUnit::Nanosecond, 1_786_708_800_123_456_789, false),
            "2026-08-14 12:00:00.123456789"
        );
        // Pre-epoch floors toward earlier days, never toward zero.
        assert_eq!(timestamp_text(us, -500_000, false), "1969-12-31 23:59:59.5");
        // BC years and the sentinels.
        // epoch_us(TIMESTAMP '-0001-01-01 01:02:03') per DuckDB v1.5.4.
        assert_eq!(
            timestamp_text(us, -62_198_751_477_000_000, false),
            "0002-01-01 (BC) 01:02:03"
        );
        assert_eq!(timestamp_text(us, i64::MAX, false), "infinity");
        assert_eq!(timestamp_text(us, -i64::MAX, false), "-infinity");
    }

    #[test]
    fn dates_render_iso_with_bc_and_sentinels() {
        assert_eq!(date_text(20_679), "2026-08-14");
        assert_eq!(date_text(0), "1970-01-01");
        assert_eq!(date_text(-719_162), "0001-01-01");
        assert_eq!(date_text(-719_528), "0001-01-01 (BC)");
        assert_eq!(date_text(i32::MAX), "infinity");
        assert_eq!(date_text(-i32::MAX), "-infinity");
    }

    #[test]
    fn times_trim_trailing_fraction_zeros() {
        assert_eq!(time_text(TimeUnit::Microsecond, 46_923_000_000), "13:02:03");
        assert_eq!(
            time_text(TimeUnit::Microsecond, 46_923_500_000),
            "13:02:03.5"
        );
    }

    #[test]
    fn intervals_speak_duckdbs_component_grammar() {
        let us = 1_000; // nanos per micro
        assert_eq!(interval_text(0, 3, 0), "3 days");
        assert_eq!(interval_text(0, 1, 0), "1 day");
        assert_eq!(interval_text(14, 0, 0), "1 year 2 months");
        assert_eq!(
            interval_text(14, 3, 14_706_789_000 * us),
            "1 year 2 months 3 days 04:05:06.789"
        );
        assert_eq!(interval_text(0, -3, 0), "-3 days");
        assert_eq!(
            interval_text(-1, -2, -3_000_000 * us),
            "-1 month -2 days -00:00:03"
        );
        assert_eq!(
            interval_text(1, -3, -14_400_000_000 * us),
            "1 month -3 days -04:00:00"
        );
        assert_eq!(interval_text(0, 0, 0), "00:00:00");
        assert_eq!(interval_text(0, 0, 90_000_000 * us), "00:01:30");
        assert_eq!(interval_text(0, 0, 93_660_000_000 * us), "26:01:00");
        assert_eq!(interval_text(0, 0, -500_000 * us), "-00:00:00.5");
        assert_eq!(interval_text(0, 0, 1_500_000 * us), "00:00:01.5");
    }

    #[test]
    fn nested_quoting_follows_the_lookup_table_rule() {
        // Bare: no structural byte, no edge whitespace, not NULL.
        assert_eq!(nested_text(&Value::Text("abc".into())), "abc");
        assert_eq!(nested_text(&Value::Text("y y".into())), "y y");
        assert_eq!(nested_text(&Value::Text("a\\b".into())), "a\\b");
        assert_eq!(nested_text(&Value::Text("nul\nline".into())), "nul\nline");
        // Quoted: structural bytes, edges, the NULL word, emptiness.
        assert_eq!(nested_text(&Value::Text("a,b".into())), "'a,b'");
        assert_eq!(nested_text(&Value::Text("a:b".into())), "'a:b'");
        assert_eq!(nested_text(&Value::Text("(x)".into())), "'(x)'");
        assert_eq!(nested_text(&Value::Text("ab=c".into())), "'ab=c'");
        assert_eq!(nested_text(&Value::Text("it's".into())), r"'it\'s'");
        assert_eq!(nested_text(&Value::Text(r"a\b,c".into())), r"'a\\b,c'");
        assert_eq!(nested_text(&Value::Text(String::new())), "''");
        assert_eq!(nested_text(&Value::Text(" lead".into())), "' lead'");
        assert_eq!(nested_text(&Value::Text("trail ".into())), "'trail '");
        assert_eq!(nested_text(&Value::Text("\tlead".into())), "'\tlead'");
        assert_eq!(nested_text(&Value::Text("nUlL".into())), "'nUlL'");
        // Temporals quote by their own content; dates only when BC's
        // parenthesis appears.
        assert_eq!(
            nested_text(&Value::Timestamp(
                TimeUnit::Microsecond,
                1_786_708_800_000_000
            )),
            "'2026-08-14 12:00:00'"
        );
        assert_eq!(nested_text(&Value::Date32(20_679)), "2026-08-14");
        assert_eq!(nested_text(&Value::Date32(-719_528)), "'0001-01-01 (BC)'");
        assert_eq!(
            nested_text(&Value::Interval {
                months: 0,
                days: 3,
                nanos: 0
            }),
            "3 days"
        );
        assert_eq!(
            nested_text(&Value::Interval {
                months: 0,
                days: 0,
                nanos: 14_706_000_000 * 1_000
            }),
            "'04:05:06'"
        );
    }

    #[test]
    fn containers_render_as_duckdb_prints_them() {
        use duckdb::types::OrderedMap;
        let strct = Value::Struct(OrderedMap::from(vec![
            ("k".to_owned(), Value::Int(1)),
            ("s".to_owned(), Value::Text("a,b".into())),
        ]));
        assert_eq!(raw_text(&strct), "{'k': 1, 's': 'a,b'}");

        let map = Value::Map(OrderedMap::from(vec![
            (Value::Int(1), Value::Text("a".into())),
            (Value::Int(2), Value::Text("b".into())),
        ]));
        assert_eq!(raw_text(&map), "{1=a, 2=b}");

        let list = Value::List(vec![Value::Int(1), Value::Null]);
        assert_eq!(raw_text(&list), "[1, NULL]");

        // A union member writes raw even where the rule would quote.
        assert_eq!(
            raw_text(&Value::Union(Box::new(Value::Text("a,b".into())))),
            "a,b"
        );

        // Blob: printable bytes raw, the rest \xHH uppercase; both quote
        // bytes escape (DuckDB v1.5.4: BLOB '''' renders \x27).
        assert_eq!(raw_text(&Value::Blob(vec![0xAA, 0x42])), r"\xAAB");
        assert_eq!(raw_text(&Value::Blob(vec![b'\'', b'"'])), r"\x27\x22");
        assert_eq!(nested_text(&Value::Blob(vec![b'a', b',', b'b'])), "'a,b'");
    }
}
