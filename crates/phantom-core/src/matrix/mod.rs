//! The Matrix protocol types phantom is built on: events, the PDUs that
//! carry them, and state resolution.

pub mod event;
pub mod pdu;
pub mod state_res;

pub use self::{
    event::Event,
    pdu::{Pdu, PduBuilder, PduCount, PduEvent, PduId, RawPduId, StateKey},
    state_res::{EventTypeExt, RoomVersion, StateMap, TypeStateKey},
};
