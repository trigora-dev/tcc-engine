// Copyright (c) 2026 Trigora, Inc.
// SPDX-License-Identifier: BUSL-1.1
// See LICENSE for full terms.

//! Language-specific arithmetic. Unsupported mixes are type errors.
//! Python divide-by-zero and out-of-range indexes raise; they do not become
//! non-finite numbers.

use tcc_ir::LANGUAGE_SEMANTICS_PY;
use tcc_state::Value;

pub enum Arith {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Neg,
    Pow,
    FloorDiv,
}

pub enum ArithFail {
    Type(&'static str),
    Raise(&'static str),
}

pub fn apply_arith(
    lang: &str,
    op: Arith,
    left: Value,
    right: Option<Value>,
) -> Result<Value, ArithFail> {
    if lang == LANGUAGE_SEMANTICS_PY {
        py_arith(op, left, right)
    } else {
        ts_arith(op, left, right)
    }
}

fn ts_arith(op: Arith, left: Value, right: Option<Value>) -> Result<Value, ArithFail> {
    match op {
        Arith::Neg => {
            let number = ts_number(left)?;
            Ok(Value::Number(-number))
        }
        Arith::Add => {
            let right = right.ok_or(ArithFail::Type("add requires two operands"))?;
            ts_add(left, right)
        }
        Arith::Sub | Arith::Mul | Arith::Div | Arith::Rem | Arith::Pow => {
            let right = right.ok_or(ArithFail::Type("arithmetic requires two numbers"))?;
            let left_n = ts_number(left)?;
            let right_n = ts_number(right)?;
            let number = match op {
                Arith::Sub => left_n - right_n,
                Arith::Mul => left_n * right_n,
                Arith::Div => left_n / right_n,
                Arith::Rem => left_n % right_n,
                Arith::Pow => js_pow(left_n, right_n),
                _ => unreachable!(),
            };
            Ok(Value::Number(number))
        }
        Arith::FloorDiv => Err(ArithFail::Type(
            "floor division is not a JavaScript operator",
        )),
    }
}

fn ts_add(left: Value, right: Value) -> Result<Value, ArithFail> {
    match (&left, &right) {
        (Value::Number(left_n), Value::Number(right_n)) => Ok(Value::Number(left_n + right_n)),
        (Value::String(left_s), _) => {
            Ok(Value::String(format!("{left_s}{}", ts_stringify(&right)?)))
        }
        (_, Value::String(right_s)) => {
            Ok(Value::String(format!("{}{right_s}", ts_stringify(&left)?)))
        }
        _ => Err(ArithFail::Type("unsupported + operands")),
    }
}

fn ts_stringify(value: &Value) -> Result<String, ArithFail> {
    match value {
        Value::String(text) => Ok(text.clone()),
        Value::Number(number) => Ok(js_number_string(*number)),
        Value::Bool(true) => Ok("true".into()),
        Value::Bool(false) => Ok("false".into()),
        Value::Null => Ok("null".into()),
        Value::Undefined => Ok("undefined".into()),
        _ => Err(ArithFail::Type("unsupported + operands")),
    }
}

fn js_number_string(number: f64) -> String {
    if number.is_nan() {
        "NaN".into()
    } else if number.is_infinite() && number.is_sign_positive() {
        "Infinity".into()
    } else if number.is_infinite() {
        "-Infinity".into()
    } else if number == 0.0 && number.is_sign_negative() {
        "0".into()
    } else {
        // Match the usual JS decimal form for finite values the corpus uses.
        let text = format!("{number}");
        text
    }
}

fn ts_number(value: Value) -> Result<f64, ArithFail> {
    match value {
        Value::Number(number) => Ok(number),
        _ => Err(ArithFail::Type("arithmetic requires numbers")),
    }
}

fn py_arith(op: Arith, left: Value, right: Option<Value>) -> Result<Value, ArithFail> {
    match op {
        Arith::Neg => {
            let number = py_number(left)?;
            Ok(Value::Number(-number))
        }
        Arith::Add => {
            let right = right.ok_or(ArithFail::Type("add requires two operands"))?;
            match (&left, &right) {
                (Value::String(left_s), Value::String(right_s)) => {
                    Ok(Value::String(format!("{left_s}{right_s}")))
                }
                _ => {
                    let left_n = py_number(left)?;
                    let right_n = py_number(right)?;
                    Ok(Value::Number(left_n + right_n))
                }
            }
        }
        Arith::Sub | Arith::Mul | Arith::Div | Arith::Rem | Arith::Pow | Arith::FloorDiv => {
            let right = right.ok_or(ArithFail::Type("arithmetic requires two operands"))?;
            let left_n = py_number(left)?;
            let right_n = py_number(right)?;
            if matches!(op, Arith::Div | Arith::Rem | Arith::FloorDiv) && right_n == 0.0 {
                return Err(ArithFail::Raise("ZeroDivisionError"));
            }
            let number = match op {
                Arith::Sub => left_n - right_n,
                Arith::Mul => left_n * right_n,
                Arith::Div => left_n / right_n,
                Arith::Rem => py_mod(left_n, right_n),
                Arith::Pow => py_pow(left_n, right_n)?,
                Arith::FloorDiv => py_floor_div(left_n, right_n),
                _ => unreachable!(),
            };
            Ok(Value::Number(number))
        }
    }
}

fn py_number(value: Value) -> Result<f64, ArithFail> {
    match value {
        Value::Number(number) => Ok(number),
        Value::Bool(true) => Ok(1.0),
        Value::Bool(false) => Ok(0.0),
        _ => Err(ArithFail::Type("arithmetic requires numbers")),
    }
}

fn js_pow(base: f64, exp: f64) -> f64 {
    if exp == 0.0 {
        return 1.0;
    }
    if exp.is_nan() || base.is_nan() {
        return f64::NAN;
    }
    let abs_base = base.abs();
    if exp.is_infinite() {
        if abs_base > 1.0 {
            return if exp.is_sign_positive() {
                f64::INFINITY
            } else {
                0.0
            };
        }
        if abs_base == 1.0 {
            return f64::NAN;
        }
        return if exp.is_sign_positive() {
            0.0
        } else {
            f64::INFINITY
        };
    }
    if base.is_infinite() {
        if exp > 0.0 {
            return if base.is_sign_negative() && is_odd_integer(exp) {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            };
        }
        return if base.is_sign_negative() && is_odd_integer(exp) {
            -0.0
        } else {
            0.0
        };
    }
    if base == 0.0 {
        if exp > 0.0 {
            return if base.is_sign_negative() && is_odd_integer(exp) {
                -0.0
            } else {
                0.0
            };
        }
        return if base.is_sign_negative() && is_odd_integer(exp) {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }
    if base < 0.0 && !is_integer(exp) {
        return f64::NAN;
    }
    signed_pow(base, exp)
}

fn py_pow(base: f64, exp: f64) -> Result<f64, ArithFail> {
    if exp == 0.0 {
        return Ok(1.0);
    }
    if base == 0.0 && exp < 0.0 {
        return Err(ArithFail::Raise("ZeroDivisionError"));
    }
    if base.is_sign_negative() && exp.is_finite() && !is_integer(exp) {
        return Err(ArithFail::Type("unsupported numeric result"));
    }
    Ok(signed_pow(base, exp))
}

fn py_floor_div(left: f64, right: f64) -> f64 {
    let modulus = left % right;
    let mut quotient = (left - modulus) / right;
    if modulus != 0.0 && right.is_sign_negative() != modulus.is_sign_negative() {
        quotient -= 1.0;
    }
    quotient
}

fn signed_pow(base: f64, exp: f64) -> f64 {
    let magnitude = base.abs().powf(exp.abs());
    let mut magnitude = if exp.is_sign_negative() {
        1.0 / magnitude
    } else {
        magnitude
    };
    if base.is_sign_negative() && is_odd_integer(exp) {
        magnitude = -magnitude;
    }
    magnitude
}

fn is_integer(number: f64) -> bool {
    number.is_finite() && number.fract() == 0.0
}

fn is_odd_integer(number: f64) -> bool {
    is_integer(number) && number % 2.0 != 0.0
}

fn py_mod(left: f64, right: f64) -> f64 {
    let mut remainder = left % right;
    if remainder != 0.0 && remainder.is_sign_negative() != right.is_sign_negative() {
        remainder += right;
    }
    remainder
}

pub fn index_number(lang: &str, value: Value) -> Result<i64, ArithFail> {
    let number = if lang == LANGUAGE_SEMANTICS_PY {
        py_number(value)?
    } else {
        ts_number(value)?
    };
    if !number.is_finite() || number.fract() != 0.0 {
        return Err(ArithFail::Type("index must be an integer"));
    }
    if number.abs() > i64::MAX as f64 {
        return Err(ArithFail::Type("index is out of range"));
    }
    Ok(number as i64)
}
