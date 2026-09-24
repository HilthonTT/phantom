use std::sync::{Arc, Mutex};

use super::{
    super::{Level, fmt as fmt_log},
    Closure, Data,
};
use crate::Result;

pub fn fmt_html<S>(out: Arc<Mutex<S>>) -> Box<Closure>
where
    S: std::fmt::Write + Send + 'static,
{
    fmt(fmt_log::html, out)
}

pub fn fmt_markdown<S>(out: Arc<Mutex<S>>) -> Box<Closure>
where
    S: std::fmt::Write + Send + 'static,
{
    fmt(fmt_log::markdown, out)
}

pub fn fmt<F, S>(fun: F, out: Arc<Mutex<S>>) -> Box<Closure>
where
    F: Fn(&mut S, &Level, &str, &str) -> Result<()> + Send + Sync + Copy + 'static,
    S: std::fmt::Write + Send + 'static,
{
    Box::new(move |data: Data<'_>| {
        let mut out = out.lock().expect("locked for writing");

        _ = fun(&mut out, &data.level(), data.span_name(), data.message());
    })
}
