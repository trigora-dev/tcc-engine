use std::collections::BTreeMap;

/// TypeScript-subset runtime values. JavaScript `number` is IEEE-754 f64.
/// Later languages must not silently reuse this for Python integers.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Object(BTreeMap<String, Value>),
    Array(Vec<Value>),
}
