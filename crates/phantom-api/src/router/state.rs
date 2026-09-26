use std::{ops::Deref, sync::Arc};

use phantom_service::Services;

#[derive(Clone)]
pub struct State(Arc<Services>);

impl State {
    #[must_use]
    pub fn new(services: Arc<Services>) -> Self {
        Self(services)
    }
}

impl Deref for State {
    type Target = Services;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
