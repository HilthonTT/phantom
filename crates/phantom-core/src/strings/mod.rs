pub mod between;
pub mod quote;
pub mod split;
pub mod unquote;

pub const EMPTY: &str = "";

pub use self::{
    between::Between, quote::Unquoted, split::SplitInfallible, unquote::Unquote,
};
