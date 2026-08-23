//! Persistence layer for the phantom homeserver.
//!
//! This crate owns the on-disk state: the key-value engine, the typed maps
//! layered over it, and the (de)serialization that turns Matrix types into
//! keys and values. Everything above it — services, the API surface — talks to
//! storage only through the handles exported here, so the choice of engine
//! stays an implementation detail of this crate.
