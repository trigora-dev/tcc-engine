// Copyright (c) 2026 Trigora, Inc.
// SPDX-License-Identifier: BUSL-1.1
// See LICENSE for full terms.

//! Types and durable operations for `cargo check` of the Rust subset.
//!
//! The lowerer recognizes the call syntax. These bodies exist so rustc can
//! move-check and typecheck authoring source. They are not the implementation
//! that runs. Name parameters are `&'static str` only here. That is not borrow
//! support in user code.

use std::marker::PhantomData;

pub struct Vec<T> {
    items: std::vec::Vec<T>,
    _type: PhantomData<T>,
}

impl<T> Default for Vec<T> {
    fn default() -> Self {
        Self {
            items: std::vec::Vec::new(),
            _type: PhantomData,
        }
    }
}

impl<T> Vec<T> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, value: T) {
        self.items.push(value);
    }

    pub fn len(&self) -> f64 {
        self.items.len() as f64
    }
}

impl<T> std::ops::Index<f64> for Vec<T> {
    type Output = T;

    fn index(&self, index: f64) -> &Self::Output {
        &self.items[index as usize]
    }
}

impl<T> IntoIterator for Vec<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter()
    }
}

pub async fn wait_for_event<T>(_name: &'static str) -> Result<T, String> {
    std::future::pending().await
}

pub async fn effect<T>(_key: &'static str, _body: impl FnOnce() -> T) -> Result<T, String> {
    std::future::pending().await
}

pub async fn sleep(_duration_ms: f64) -> Result<(), String> {
    std::future::pending().await
}

pub async fn invoke<T, A>(_child: &'static str, _args: A) -> Result<T, String> {
    std::future::pending().await
}

pub async fn join<T>(
    _left: impl std::future::Future<Output = Result<T, String>>,
    _right: impl std::future::Future<Output = Result<T, String>>,
) -> Result<Vec<T>, String> {
    std::future::pending().await
}

pub async fn race<T>(
    _left: impl std::future::Future<Output = Result<T, String>>,
    _right: impl std::future::Future<Output = Result<T, String>>,
) -> Result<T, String> {
    std::future::pending().await
}
