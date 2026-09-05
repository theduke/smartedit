//! A catalogue of Rust declarations.
#![allow(dead_code)]

use std::fmt::{self, Display};
use std::collections::*;

/// The selected answer.
pub const ANSWER: usize = 42;
pub static mut COUNTER: i32 = 0;

#[derive(Clone, Debug)]
pub struct Store<T> {
    entries: Vec<T>,
}

pub enum Choice {
    First,
    Number(i64),
    Named { label: String },
}

pub union Bits {
    integer: u32,
    decimal: f32,
}

pub type Handler = fn(&str) -> usize;

pub trait Render {
    const NAME: &'static str;
    type Output;
    fn render(&self) -> Self::Output;
}

impl<T> Store<T> {
    pub fn new() -> Self {
        Self { entries: Vec::new() }
    }

    pub fn push(&mut self, entry: T) {
        self.entries.push(entry);
    }
}

impl<T: Display> Render for Store<T> {
    const NAME: &'static str = "store";
    type Output = String;

    fn render(&self) -> Self::Output {
        format!("{} entries", self.entries.len())
    }
}

macro_rules! announce {
    ($value:expr) => { println!("{value}") };
}

extern "C" {
    fn puts(value: *const i8) -> i32;
}

pub mod nested {
    /// A background worker.
    pub struct Worker;

    impl Worker {
        pub fn launch(&self) {
            fn local() -> &'static str { "ready" }
            announce!(local());
        }
    }
}
