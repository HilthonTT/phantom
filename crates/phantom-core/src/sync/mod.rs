//! Synchronization primitives phantom needs and `std` does not have.

pub mod mutex_map;
pub mod two_phase_counter;

pub use self::{
    mutex_map::{Guard as MutexMapGuard, MutexMap},
    two_phase_counter::{Counter as TwoPhaseCounter, Permit as CounterPermit},
};
