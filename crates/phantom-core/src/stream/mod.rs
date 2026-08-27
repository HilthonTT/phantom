//! Extensions to [`futures::Stream`] and [`futures::TryStream`].

mod aggregate;
mod band;
mod broadband;
mod cloned;
mod expect;
mod ignore;
mod iter_stream;
mod ready;
mod try_aggregate;
mod try_broadband;
mod try_parallel;
mod try_ready;
mod try_wideband;
mod wideband;

pub use self::{
    aggregate::Aggregate,
    band::{
        AMPLIFICATION_LIMIT, WIDTH_LIMIT, automatic_amplification, automatic_width,
        set_amplification, set_width,
    },
    broadband::BroadbandExt,
    cloned::Cloned,
    expect::TryExpect,
    ignore::TryIgnore,
    iter_stream::IterStream,
    ready::ReadyExt,
    try_aggregate::TryAggregate,
    try_broadband::TryBroadbandExt,
    try_parallel::TryParallelExt,
    try_ready::TryReadyExt,
    try_wideband::TryWidebandExt,
    wideband::WidebandExt,
};
