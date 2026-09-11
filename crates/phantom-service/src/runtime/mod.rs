pub mod contract;
pub mod manager;
pub mod registry;
pub mod services;

pub use self::{
    contract::{Args, Service},
    manager::Manager,
    registry::{Dep, Map, add, get, make_name, try_get},
    services::Services,
};
