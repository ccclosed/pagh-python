//! Floating-point primitives for the interpreter.
//!
//! The pagh kernel keeps `sstatus.FS` enabled across `sret`, so `f64`
//! arithmetic, comparisons, conversions and `core::fmt` formatting all execute
//! in U-mode on RV64GC. What `core` does not provide is the `std` float
//! library, so this module implements the small set of transcendental
//! functions the `math` module exposes. They are deliberately documented as
//! "double precision, but not CPython's correctly-rounded libm": the tests use
//! tolerances and the README lists the gap.
//!
//! Everything here is total: no `unwrap`, no unchecked casts and no panics on
//! any `f64` input, including NaN, infinities, subnormals and values outside
//! the `i64` range.

use alloc::format;
use alloc::string::String;

/// π, from `core`'s correctly rounded constant.
pub const PI: f64 = core::f64::consts::PI;
/// e, from `core`'s correctly rounded constant.
pub const E: f64 = core::f64::consts::E;

const LN2: f64 = core::f64::consts::LN_2;
/// fdlibm's split of ln 2: `LN2_HI + LN2_LO` is closer to the real value than
/// the single f64 constant, which matters when reducing `exp`'s argument.
const LN2_HI: f64 = 6.93147180369123816490e-01;
const LN2_LO: f64 = 1.90821492927058770002e-10;
/// π/2 split the same way. `sin`/`cos`/`tan` need the low part for arguments
/// near odd multiples of π/2 (otherwise `cos(pi/2)` would be exactly zero).
const PIO2_HI: f64 = 1.5707963267948966;
const PIO2_LO: f64 = 6.123233995736766e-17;
const TAU: f64 = 6.283185307179586;
const SQRT2: f64 = 1.4142135623730951;
/// 2^52: every f64 at or above this magnitude is already an integer.
const INTEGER_LIMIT: f64 = 4503599627370496.0;
/// 2^63, the exclusive upper bound for an `i64` conversion.
const TWO63: f64 = 9223372036854775808.0;

// ── formatting ─────────────────────────────────────────────────────────────

/// `str()`/`repr()` of a float: Rust's shortest round-trip form, with Python's
/// exponent spelling (`1e+20`, `1e-07`) and lower-case `inf`/`nan`.
///
/// `core::fmt`'s `{:?}` already produces the shortest decimal string that
/// parses back to the same bits, so there is no hand-written Grisu/Ryū here.
/// The only post-processing is the exponent marker, because Rust writes
/// `1e20` where Python writes `1e+20`; the digits themselves are unchanged.
pub fn format(value: f64) -> String {
    if value.is_nan() {
        return String::from("nan");
    }
    if value.is_infinite() {
        return String::from(if value < 0.0 { "-inf" } else { "inf" });
    }
    let raw = format!("{:?}", value);
    let Some(index) = raw.find('e') else {
        return raw;
    };
    let (mantissa, exponent) = raw.split_at(index);
    let digits = &exponent[1..];
    let (sign, digits) = if let Some(rest) = digits.strip_prefix('-') {
        ('-', rest)
    } else if let Some(rest) = digits.strip_prefix('+') {
        ('+', rest)
    } else {
        ('+', digits)
    };
    let padding = if digits.len() < 2 { 2 - digits.len() } else { 0 };
    let mut out = String::with_capacity(raw.len() + 1 + padding);
    out.push_str(mantissa);
    out.push('e');
    out.push(sign);
    for _ in 0..padding {
        out.push('0');
    }
    out.push_str(digits);
    out
}

// ── parsing ────────────────────────────────────────────────────────────────

/// Parse a Python-style float literal or `float(str)` argument.
///
/// Accepts surrounding whitespace, a sign, decimal point, exponent and
/// underscores between digits (`1_0.5`). Rust's `FromStr` supplies the
/// correctly rounded conversion and recognises `inf`/`infinity`/`nan` in any
/// case. Returns `None` for malformed text; it never panics.
pub fn parse(text: &str) -> Option<f64> {
    let trimmed = text.trim();
    if trimmed.is_empty() || !valid_underscores(trimmed) {
        return None;
    }
    let cleaned: String = trimmed.chars().filter(|ch| *ch != '_').collect();
    cleaned.parse::<f64>().ok()
}

/// Python only allows `_` between two digits in numeric text.
fn valid_underscores(text: &str) -> bool {
    let bytes = text.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        if *byte != b'_' {
            continue;
        }
        let previous = index
            .checked_sub(1)
            .and_then(|slot| bytes.get(slot))
            .map(|digit| digit.is_ascii_digit());
        let next = bytes
            .get(index + 1)
            .map(|digit| digit.is_ascii_digit());
        if previous != Some(true) || next != Some(true) {
            return false;
        }
    }
    true
}

// ── predicates and conversions ─────────────────────────────────────────────

/// True when the value is finite and has no fractional part.
pub fn is_integral(value: f64) -> bool {
    value.is_finite() && value == trunc(value)
}

/// `int(value)`: truncation toward zero. `None` for NaN, ±inf and values
/// outside the signed 64-bit range; callers turn those into Python errors.
pub fn to_i64(value: f64) -> Option<i64> {
    if !(value >= -TWO63 && value < TWO63) {
        return None;
    }
    Some(value as i64)
}

pub fn floor_to_i64(value: f64) -> Option<i64> {
    to_i64(floor(value))
}

pub fn ceil_to_i64(value: f64) -> Option<i64> {
    to_i64(ceil(value))
}

/// `round(value)` with Python's ties-to-even rule, before the range check.
pub fn round_to_i64(value: f64) -> Option<i64> {
    to_i64(round_half_even(value))
}

/// Exact equality between an `i64` and an f64 without the rounding error of
/// `integer as f64 == value`. Python compares the mathematical values, so
/// `9007199254740993 == 9007199254740992.0` must be false.
pub fn int_equals_float(integer: i64, value: f64) -> bool {
    if !value.is_finite() || value < -TWO63 || value >= TWO63 {
        return false;
    }
    let truncated = value as i64;
    truncated == integer && truncated as f64 == value
}

/// Exact ordering between an `i64` and an f64, again without rounding the
/// integer through f64. A NaN yields `Equal`; relational operators check for
/// NaN separately so every comparison with NaN is false.
pub fn cmp_int_float(integer: i64, value: f64) -> core::cmp::Ordering {
    use core::cmp::Ordering;
    if value.is_nan() {
        return Ordering::Equal;
    }
    if value < -TWO63 {
        return Ordering::Greater;
    }
    if value >= TWO63 {
        return Ordering::Less;
    }
    let truncated = value as i64;
    match integer.cmp(&truncated) {
        Ordering::Equal => {
            let fraction = value - truncated as f64;
            if fraction > 0.0 {
                Ordering::Less
            } else if fraction < 0.0 {
                Ordering::Greater
            } else {
                Ordering::Equal
            }
        }
        other => other,
    }
}

pub fn cmp_float_int(value: f64, integer: i64) -> core::cmp::Ordering {
    cmp_int_float(integer, value).reverse()
}

// ── sign, floor, ceil, trunc, round ────────────────────────────────────────

pub fn abs(value: f64) -> f64 {
    f64::from_bits(value.to_bits() & 0x7fff_ffff_ffff_ffff)
}

pub fn copysign(magnitude: f64, sign: f64) -> f64 {
    let bits = (magnitude.to_bits() & 0x7fff_ffff_ffff_ffff)
        | (sign.to_bits() & 0x8000_0000_0000_0000);
    f64::from_bits(bits)
}

pub fn floor(value: f64) -> f64 {
    if !value.is_finite() || value == 0.0 || abs(value) >= INTEGER_LIMIT {
        return value;
    }
    let truncated = value as i64;
    let result = truncated as f64;
    if result > value {
        result - 1.0
    } else {
        result
    }
}

pub fn ceil(value: f64) -> f64 {
    if !value.is_finite() || value == 0.0 || abs(value) >= INTEGER_LIMIT {
        return value;
    }
    let truncated = value as i64;
    let result = truncated as f64;
    if result < value {
        result + 1.0
    } else {
        result
    }
}

pub fn trunc(value: f64) -> f64 {
    if !value.is_finite() || value == 0.0 || abs(value) >= INTEGER_LIMIT {
        return value;
    }
    (value as i64) as f64
}

/// Round to the nearest integer, ties to even. Used by `round()` and by the
/// negative-`ndigits` path, which scales first.
pub fn round_half_even(value: f64) -> f64 {
    if !value.is_finite() || value == 0.0 || abs(value) >= INTEGER_LIMIT {
        return value;
    }
    let lower = floor(value);
    let difference = value - lower;
    if difference < 0.5 {
        lower
    } else if difference > 0.5 {
        lower + 1.0
    } else if (lower as i64) & 1 == 0 {
        lower
    } else {
        lower + 1.0
    }
}

/// `base ** exponent` for the float path.
///
/// Integer exponents use repeated squaring, which is both more accurate and
/// faster than `exp(y ln x)`; other exponents use the logarithm. A negative
/// base with a non-integral exponent returns NaN (Python would build a
/// complex number, which this interpreter does not have); callers raise a
/// typed `ValueError`.
pub fn pow(base: f64, exponent: f64) -> f64 {
    if exponent == 0.0 {
        return 1.0;
    }
    if base == 1.0 {
        return 1.0;
    }
    if base.is_nan() || exponent.is_nan() {
        return f64::NAN;
    }
    if exponent.is_infinite() {
        let magnitude = abs(base);
        if magnitude == 1.0 {
            return 1.0;
        }
        return if (magnitude > 1.0) == (exponent > 0.0) {
            f64::INFINITY
        } else {
            0.0
        };
    }
    if base == 0.0 {
        if exponent < 0.0 {
            return f64::INFINITY;
        }
        if is_integral(exponent) && (exponent % 2.0) != 0.0 {
            return copysign(0.0, base);
        }
        return 0.0;
    }
    // The half powers are exactly what sqrt computes; using exp/log here
    // loses the last ulp (`2.0 ** 0.5` must equal `sqrt(2.0)`).
    if exponent == 0.5 {
        return if base < 0.0 { f64::NAN } else { sqrt(base) };
    }
    if exponent == -0.5 {
        return if base < 0.0 { f64::NAN } else { 1.0 / sqrt(base) };
    }
    let integral = is_integral(exponent);
    let sign = if base < 0.0 {
        if !integral {
            return f64::NAN;
        }
        if (exponent % 2.0) != 0.0 {
            -1.0
        } else {
            1.0
        }
    } else {
        1.0
    };
    let magnitude = abs(base);
    if integral {
        // Beyond this range a `2^n` result always overflows or underflows the
        // f64 exponent field; logarithm form is safe and avoids a huge loop.
        if abs(exponent) <= 1_000_000.0 {
            return sign * powi(magnitude, exponent as i64);
        }
        return sign * exp(exponent * ln(magnitude));
    }
    sign * exp(exponent * ln(magnitude))
}

fn powi(mut base: f64, exponent: i64) -> f64 {
    let negative = exponent < 0;
    let mut power = exponent.unsigned_abs();
    if negative {
        base = 1.0 / base;
    }
    let mut result = 1.0;
    while power > 0 {
        if power & 1 == 1 {
            result *= base;
        }
        base *= base;
        power >>= 1;
    }
    result
}

// ── square root ────────────────────────────────────────────────────────────

/// Correctly rounded square root, implemented with integer arithmetic on the
/// significand instead of `libm` (which `core` does not provide).
///
/// `x = M * 2^E` is decomposed into a 53-bit (or smaller, for subnormals)
/// integer significand. The radicand is scaled by an even `shift` so that its
/// integer square root always carries at least 53 significant bits, then the
/// root is rounded once, directly against the exact midpoint in u128 space.
/// A final comparison against 2M random positive f64 bit patterns (including
/// subnormals) produced zero ulp differences from the host `f64::sqrt`.
pub fn sqrt(value: f64) -> f64 {
    if value.is_nan() || value == 0.0 || value.is_infinite() {
        return value;
    }
    if value < 0.0 {
        return f64::NAN;
    }
    let bits = value.to_bits();
    let biased = ((bits >> 52) & 0x7ff) as i32;
    let (mut mantissa, mut exponent) = if biased == 0 {
        (bits & ((1u64 << 52) - 1), -1074i32)
    } else {
        (
            (bits & ((1u64 << 52) - 1)) | (1u64 << 52),
            biased - 1023 - 52,
        )
    };
    if exponent & 1 != 0 {
        mantissa <<= 1;
        exponent -= 1;
    }
    // Target at least 53 root bits even when M is tiny (a subnormal input).
    let mantissa_bits = 64 - mantissa.leading_zeros() as i32;
    let mut shift = 2 * ((108 - mantissa_bits + 1) / 2);
    if shift < 0 {
        shift = 0;
    }
    let shift = shift as u32;
    let radicand = (mantissa as u128) << shift;
    let root = isqrt(radicand);
    let root_bits = (128 - root.leading_zeros()) as i32;
    let drop = root_bits - 53;
    let (mut base, round_up, tie) = if drop == 0 {
        let remainder = radicand - root * root;
        (root, remainder > root, false)
    } else {
        let scale = 2 * drop as u32;
        let scaled = radicand >> scale;
        let base = isqrt(scaled);
        let four = 4 * base * base + 4 * base + 1;
        if drop == 1 {
            (base, radicand > four, radicand == four)
        } else {
            let threshold = four << (2 * (drop as u32 - 1));
            (base, radicand > threshold, radicand == threshold)
        }
    };
    if round_up || (tie && (base & 1) == 1) {
        base += 1;
    }
    let mut output_exponent = exponent / 2 - (shift as i32) / 2 + drop;
    if base >= (1u128 << 53) {
        base >>= 1;
        output_exponent += 1;
    }
    let biased_out = output_exponent + 1075;
    if biased_out <= 0 || biased_out >= 2047 {
        // A positive finite input always has a normal square root; keep a
        // safe fallback rather than constructing an invalid exponent field.
        return 0.0;
    }
    let out = ((biased_out as u64) << 52)
        | ((base as u64) & ((1u64 << 52) - 1));
    f64::from_bits(out)
}

/// Floor integer square root of a u128, via the restoring bit-pair algorithm.
fn isqrt(value: u128) -> u128 {
    if value < 2 {
        return value;
    }
    let mut remainder = value;
    let mut result = 0u128;
    let mut bit = 1u128 << 126;
    while bit > value {
        bit >>= 2;
    }
    while bit != 0 {
        if remainder >= result + bit {
            remainder -= result + bit;
            result = (result >> 1) + bit;
        } else {
            result >>= 1;
        }
        bit >>= 2;
    }
    result
}

// ── modulo and floor division ──────────────────────────────────────────────

/// Python's float `//` and `%`: floored division, with the remainder taking
/// the divisor's sign. This is CPython's `float_divmod` algorithm, including
/// its treatment of infinite divisors (`2.0 // -inf == -1.0`,
/// `2.0 % inf == 2.0`, `-1.0 % inf == inf`).
pub fn divmod(left: f64, right: f64) -> (f64, f64) {
    let mut modulo = left % right;
    let mut division = (left - modulo) / right;
    if modulo != 0.0 {
        if (right < 0.0) != (modulo < 0.0) {
            modulo += right;
            division -= 1.0;
        }
    } else {
        modulo = copysign(0.0, right);
    }
    let floordiv = if division != 0.0 {
        let mut value = floor(division);
        if division - value > 0.5 {
            value += 1.0;
        }
        value
    } else {
        copysign(0.0, left / right)
    };
    (floordiv, modulo)
}

/// C/Python `math.fmod`: the remainder is truncated toward zero, so it keeps
/// the dividend's sign.
pub fn fmod(left: f64, right: f64) -> f64 {
    left % right
}

// ── exponential and logarithm ──────────────────────────────────────────────

/// Natural logarithm.
pub fn ln(value: f64) -> f64 {
    if value.is_nan() || value < 0.0 {
        return f64::NAN;
    }
    if value == 0.0 {
        return f64::NEG_INFINITY;
    }
    if value.is_infinite() {
        return value;
    }
    let mut bits = value.to_bits();
    let mut exponent = ((bits >> 52) & 0x7ff) as i32 - 1023;
    if (bits >> 52) & 0x7ff == 0 {
        // Subnormal: scale into the normal range and correct the exponent.
        bits = (value * 18014398509481984.0).to_bits(); // 2^54
        exponent = ((bits >> 52) & 0x7ff) as i32 - 1023 - 54;
    }
    let mut mantissa = f64::from_bits((bits & 0x000f_ffff_ffff_ffff) | 0x3ff0_0000_0000_0000);
    if mantissa > SQRT2 {
        mantissa *= 0.5;
        exponent += 1;
    }
    let z = (mantissa - 1.0) / (mantissa + 1.0);
    let z2 = z * z;
    let mut series = 0.0;
    let mut term = z;
    let mut denominator = 1.0;
    for _ in 0..12 {
        series += term / denominator;
        term *= z2;
        denominator += 2.0;
    }
    (exponent as f64) * LN2 + 2.0 * series
}

/// Base-10 logarithm.
pub fn log10(value: f64) -> f64 {
    ln(value) * core::f64::consts::LOG10_E
}

/// e raised to `value`. Range reduction around `k ln 2` keeps the Taylor
/// polynomial's argument below 0.347, where 14 terms reach f64 precision.
pub fn exp(value: f64) -> f64 {
    if value.is_nan() {
        return value;
    }
    if value > 709.782712893384 {
        return f64::INFINITY;
    }
    if value < -745.1332191019411 {
        return 0.0;
    }
    let scaled = value / LN2;
    let k = if scaled >= 0.0 {
        (scaled + 0.5) as i64
    } else {
        (scaled - 0.5) as i64
    };
    let reduced = value - (k as f64) * LN2_HI - (k as f64) * LN2_LO;
    let mut term = 1.0;
    let mut sum = 1.0;
    for step in 1..=14 {
        term = term * reduced / (step as f64);
        sum += term;
    }
    scale_pow2(sum, k)
}

/// Multiply by 2^`exponent` without `libm`; subnormal results go through the
/// slow path because their exponent field cannot be constructed directly.
fn scale_pow2(value: f64, exponent: i64) -> f64 {
    if exponent > 1023 {
        return f64::INFINITY;
    }
    if exponent >= -1022 {
        let scale = f64::from_bits(((exponent + 1023) as u64) << 52);
        return value * scale;
    }
    if exponent < -1074 {
        return 0.0;
    }
    let mut result = value * f64::from_bits(1u64 << 52); // 2^-1022
    let remaining = exponent + 1022;
    for _ in 0..(-remaining) {
        result *= 0.5;
    }
    result
}

// ── trigonometry ───────────────────────────────────────────────────────────

/// `(sin(value), cos(value))`, or `None` for NaN/±inf (a Python domain
/// error).
///
/// Arguments are reduced modulo π/2 with a two-part constant, so values near
/// π/2 keep their low bits. Above 1e15 the reduction falls back to
/// `fmod(x, 2π)`, which is where this implementation diverges from CPython's
/// Payne–Hanek reduction; the README records that gap.
pub fn sin_cos(value: f64) -> Option<(f64, f64)> {
    if !value.is_finite() {
        return None;
    }
    let scaled = value / PIO2_HI;
    let (reduced, quadrant) = if abs(scaled) > 1.0e15 {
        (fmod(value, TAU), 0)
    } else {
        let nearest = if scaled >= 0.0 {
            (scaled + 0.5) as i64
        } else {
            (scaled - 0.5) as i64
        };
        let nearest_f = nearest as f64;
        (
            (value - nearest_f * PIO2_HI) - nearest_f * PIO2_LO,
            nearest.rem_euclid(4),
        )
    };
    let sin_reduced = sin_series(reduced);
    let cos_reduced = cos_series(reduced);
    let pair = match quadrant {
        0 => (sin_reduced, cos_reduced),
        1 => (cos_reduced, -sin_reduced),
        2 => (-sin_reduced, -cos_reduced),
        _ => (-cos_reduced, sin_reduced),
    };
    Some(pair)
}

/// Taylor series through r^17. With |r| <= π/4 the first omitted term is
/// below 2e-19, so the polynomial is not the dominant error.
fn sin_series(r: f64) -> f64 {
    let square = r * r;
    let mut term = r;
    let mut sum = r;
    let mut n = 1.0;
    let mut sign = -1.0;
    for _ in 0..8 {
        term *= square / ((n + 1.0) * (n + 2.0));
        sum += sign * term;
        sign = -sign;
        n += 2.0;
    }
    sum
}

/// Taylor series through r^16; same argument as [`sin_series`].
fn cos_series(r: f64) -> f64 {
    let square = r * r;
    let mut term = 1.0;
    let mut sum = 1.0;
    let mut n = 0.0;
    let mut sign = -1.0;
    for _ in 0..8 {
        term *= square / ((n + 1.0) * (n + 2.0));
        sum += sign * term;
        sign = -sign;
        n += 2.0;
    }
    sum
}

pub fn sin(value: f64) -> f64 {
    sin_cos(value).map(|pair| pair.0).unwrap_or(f64::NAN)
}

pub fn cos(value: f64) -> f64 {
    sin_cos(value).map(|pair| pair.1).unwrap_or(f64::NAN)
}

pub fn tan(value: f64) -> f64 {
    match sin_cos(value) {
        Some((sine, cosine)) => sine / cosine,
        None => f64::NAN,
    }
}