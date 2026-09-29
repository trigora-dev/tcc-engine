// Copyright (c) 2026 Trigora, Inc.
// SPDX-License-Identifier: BUSL-1.1
// See LICENSE for full terms.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::error::IrError;

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    Object(BTreeMap<String, Json>),
}

impl Json {
    pub fn stringify(&self) -> String {
        let mut out = String::new();
        write_json(&mut out, self);
        out
    }

    pub fn parse(input: &str) -> Result<Json, IrError> {
        let mut parser = Parser {
            bytes: input.as_bytes(),
            index: 0,
        };
        parser.skip_ws();
        let value = parser.parse_value()?;
        parser.skip_ws();
        if parser.index != parser.bytes.len() {
            return Err(IrError::InvalidEncoding("trailing data".to_string()));
        }
        Ok(value)
    }

    pub fn as_object(&self) -> Result<&BTreeMap<String, Json>, IrError> {
        match self {
            Json::Object(map) => Ok(map),
            _ => Err(IrError::InvalidEncoding("expected object".to_string())),
        }
    }

    pub fn as_str(&self) -> Result<&str, IrError> {
        match self {
            Json::String(text) => Ok(text),
            _ => Err(IrError::InvalidEncoding("expected string".to_string())),
        }
    }

    pub fn as_u64(&self) -> Result<u64, IrError> {
        match self {
            Json::Number(number) if *number >= 0.0 && number.fract() == 0.0 => Ok(*number as u64),
            _ => Err(IrError::InvalidEncoding("expected u64".to_string())),
        }
    }

    pub fn as_u32(&self) -> Result<u32, IrError> {
        u32::try_from(self.as_u64()?)
            .map_err(|_| IrError::InvalidEncoding("u32 overflow".to_string()))
    }

    pub fn as_array(&self) -> Result<&[Json], IrError> {
        match self {
            Json::Array(items) => Ok(items),
            _ => Err(IrError::InvalidEncoding("expected array".to_string())),
        }
    }

    pub fn get<'a>(map: &'a BTreeMap<String, Json>, key: &str) -> Result<&'a Json, IrError> {
        map.get(key)
            .ok_or_else(|| IrError::InvalidEncoding(format!("missing field `{key}`")))
    }
}

fn write_finite_number(out: &mut String, number: f64) {
    if number == 0.0 && number.is_sign_negative() {
        out.push_str("-0");
        return;
    }
    if number.is_finite() {
        let _ = write!(out, "{number}");
        return;
    }
    out.push_str("null");
}

fn write_json(out: &mut String, value: &Json) {
    match value {
        Json::Null => out.push_str("null"),
        Json::Bool(true) => out.push_str("true"),
        Json::Bool(false) => out.push_str("false"),
        Json::Number(number) => write_finite_number(out, *number),
        Json::String(text) => write_string(out, text),
        Json::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_json(out, item);
            }
            out.push(']');
        }
        Json::Object(map) => {
            out.push('{');
            for (index, (key, item)) in map.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_string(out, key);
                out.push(':');
                write_json(out, item);
            }
            out.push('}');
        }
    }
}

fn write_string(out: &mut String, text: &str) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => {
                let _ = write!(out, "\\u{:04x}", u32::from(ch));
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
}

struct Parser<'a> {
    bytes: &'a [u8],
    index: usize,
}

impl Parser<'_> {
    fn skip_ws(&mut self) {
        while let Some(byte) = self.bytes.get(self.index) {
            if !byte.is_ascii_whitespace() {
                break;
            }
            self.index += 1;
        }
    }

    fn parse_value(&mut self) -> Result<Json, IrError> {
        self.skip_ws();
        let byte = *self
            .bytes
            .get(self.index)
            .ok_or_else(|| IrError::InvalidEncoding("unexpected end".to_string()))?;
        match byte {
            b'n' => self.parse_literal(b"null").map(|()| Json::Null),
            b't' => self.parse_literal(b"true").map(|()| Json::Bool(true)),
            b'f' => self.parse_literal(b"false").map(|()| Json::Bool(false)),
            b'"' => self.parse_string().map(Json::String),
            b'[' => self.parse_array(),
            b'{' => self.parse_object(),
            b'-' | b'0'..=b'9' => self.parse_number(),
            _ => Err(IrError::InvalidEncoding("unexpected token".to_string())),
        }
    }

    fn parse_literal(&mut self, literal: &[u8]) -> Result<(), IrError> {
        if self.bytes.get(self.index..self.index + literal.len()) != Some(literal) {
            return Err(IrError::InvalidEncoding("invalid literal".to_string()));
        }
        self.index += literal.len();
        Ok(())
    }

    fn parse_string(&mut self) -> Result<String, IrError> {
        self.index += 1;
        let mut out = String::new();
        while let Some(&byte) = self.bytes.get(self.index) {
            match byte {
                b'"' => {
                    self.index += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.index += 1;
                    let escaped = *self
                        .bytes
                        .get(self.index)
                        .ok_or_else(|| IrError::InvalidEncoding("bad escape".to_string()))?;
                    out.push(match escaped {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            return Err(IrError::InvalidEncoding(
                                "unicode escape unsupported".to_string(),
                            ))
                        }
                        _ => return Err(IrError::InvalidEncoding("bad escape".to_string())),
                    });
                    self.index += 1;
                }
                _ => {
                    out.push(byte as char);
                    self.index += 1;
                }
            }
        }
        Err(IrError::InvalidEncoding("unterminated string".to_string()))
    }

    fn parse_array(&mut self) -> Result<Json, IrError> {
        self.index += 1;
        self.skip_ws();
        let mut items = Vec::new();
        if self.bytes.get(self.index) == Some(&b']') {
            self.index += 1;
            return Ok(Json::Array(items));
        }
        loop {
            items.push(self.parse_value()?);
            self.skip_ws();
            match self.bytes.get(self.index) {
                Some(&b',') => {
                    self.index += 1;
                }
                Some(&b']') => {
                    self.index += 1;
                    return Ok(Json::Array(items));
                }
                _ => {
                    return Err(IrError::InvalidEncoding(
                        "expected array delimiter".to_string(),
                    ))
                }
            }
        }
    }

    fn parse_object(&mut self) -> Result<Json, IrError> {
        self.index += 1;
        self.skip_ws();
        let mut map = BTreeMap::new();
        if self.bytes.get(self.index) == Some(&b'}') {
            self.index += 1;
            return Ok(Json::Object(map));
        }
        loop {
            self.skip_ws();
            if self.bytes.get(self.index) != Some(&b'"') {
                return Err(IrError::InvalidEncoding("expected object key".to_string()));
            }
            let key = self.parse_string()?;
            self.skip_ws();
            if self.bytes.get(self.index) != Some(&b':') {
                return Err(IrError::InvalidEncoding("expected colon".to_string()));
            }
            self.index += 1;
            let value = self.parse_value()?;
            map.insert(key, value);
            self.skip_ws();
            match self.bytes.get(self.index) {
                Some(&b',') => {
                    self.index += 1;
                }
                Some(&b'}') => {
                    self.index += 1;
                    return Ok(Json::Object(map));
                }
                _ => {
                    return Err(IrError::InvalidEncoding(
                        "expected object delimiter".to_string(),
                    ))
                }
            }
        }
    }

    fn parse_number(&mut self) -> Result<Json, IrError> {
        let start = self.index;
        if self.bytes.get(self.index) == Some(&b'-') {
            self.index += 1;
        }
        while matches!(self.bytes.get(self.index), Some(b'0'..=b'9')) {
            self.index += 1;
        }
        if self.bytes.get(self.index) == Some(&b'.') {
            self.index += 1;
            while matches!(self.bytes.get(self.index), Some(b'0'..=b'9')) {
                self.index += 1;
            }
        }
        if matches!(self.bytes.get(self.index), Some(b'e' | b'E')) {
            self.index += 1;
            if matches!(self.bytes.get(self.index), Some(b'+' | b'-')) {
                self.index += 1;
            }
            while matches!(self.bytes.get(self.index), Some(b'0'..=b'9')) {
                self.index += 1;
            }
        }
        let text = std::str::from_utf8(&self.bytes[start..self.index])
            .map_err(|_| IrError::InvalidEncoding("invalid number".to_string()))?;
        let number = text
            .parse::<f64>()
            .map_err(|_| IrError::InvalidEncoding("invalid number".to_string()))?;
        Ok(Json::Number(number))
    }
}
