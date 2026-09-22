mod data;

use data::Data;

use phantom_core::{Err, Result, error, rand::string, stream::IterStream};

const RANDOM_TOKEN_LENGTH: usize = 16;

pub struct Service {
    db: Data,
}
