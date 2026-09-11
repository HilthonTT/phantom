use arrayvec::ArrayString;
use rand::{RngExt, rng, seq::SliceRandom};
use std::{
    ops::Range,
    time::{Duration, SystemTime},
};

pub fn shuffle<T>(vec: &mut [T]) {
    let mut rng = rng();
    vec.shuffle(&mut rng);
}

pub fn string(length: usize) -> String {
    rng()
        .sample_iter(rand::distr::Alphanumeric)
        .take(length)
        .map(char::from)
        .collect()
}

pub fn string_from(charset: &[u8], length: usize) -> String {
    debug_assert!(!charset.is_empty(), "the charset must have something in it");
    debug_assert!(charset.is_ascii(), "the charset must be ASCII");

    let mut rng = rng();

    (0..length)
        .map(|_| char::from(charset[rng.random_range(0..charset.len())]))
        .collect()
}

#[inline]
pub fn string_array<const LENGTH: usize>() -> ArrayString<LENGTH> {
    let mut ret = ArrayString::<LENGTH>::new();
    rng()
        .sample_iter(rand::distr::Alphanumeric)
        .take(LENGTH)
        .map(char::from)
        .for_each(|c| ret.push(c));

    ret
}

#[inline]
#[must_use]
pub fn time_from_now_secs(range: Range<u64>) -> SystemTime {
    SystemTime::now()
        .checked_add(secs(range))
        .expect("range does not overflow SystemTime")
}

#[must_use]
pub fn secs(range: Range<u64>) -> Duration {
    let mut rng = rng();
    Duration::from_secs(rng.random_range(range))
}
