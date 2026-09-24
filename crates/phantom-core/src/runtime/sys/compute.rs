use std::{cell::Cell, fmt::Debug, path::PathBuf, sync::LazyLock};

use crate::{Result, is_equal_to};

type Id = usize;

type Mask = u128;
type Masks = [Mask; MASK_BITS];

const MASK_BITS: usize = 128;

static CORES_AVAILABLE: LazyLock<Mask> = LazyLock::new(|| into_mask(query_cores_available()));

static SMT_TOPOLOGY: LazyLock<Masks> = LazyLock::new(init_smt_topology);

static NODE_TOPOLOGY: LazyLock<Masks> = LazyLock::new(init_node_topology);

thread_local! {

    static CORE_AFFINITY: Cell<Mask> = Cell::default();
}

#[tracing::instrument(
	level = "debug",
	skip_all,
	fields(
		id = ?std::thread::current().id(),
		name = %std::thread::current().name().unwrap_or("None"),
		set = ?ids.clone().collect::<Vec<_>>(),
		CURRENT = %format!("[b{:b}]", CORE_AFFINITY.get()),
		AVAILABLE = %format!("[b{:b}]", *CORES_AVAILABLE),
	),
)]
pub fn set_affinity<I>(mut ids: I)
where
    I: Iterator<Item = Id> + Clone + Debug,
{
    use core_affinity::{CoreId, set_for_current};

    let n = ids.clone().count();
    let mask: Mask = ids.clone().fold(0, |mask, id| {
        debug_assert!(
            is_core_available(id),
            "setting affinity to unavailable core"
        );
        mask | (1 << id)
    });

    if n > 1 {
        set_each_for_current(ids);
    } else if n > 0 {
        set_for_current(CoreId {
            id: ids.next().expect("n > 0"),
        });
    }

    if mask.count_ones() > 0 {
        CORE_AFFINITY.replace(mask);
    }
}

pub fn get_affinity() -> impl Iterator<Item = Id> {
    from_mask(CORE_AFFINITY.get())
}

pub fn smt_siblings() -> impl Iterator<Item = Id> {
    from_mask(get_affinity().fold(0_u128, |mask, id| {
        mask | SMT_TOPOLOGY.get(id).expect("ID must not exceed max cpus")
    }))
}

pub fn node_siblings() -> impl Iterator<Item = Id> {
    from_mask(get_affinity().fold(0_u128, |mask, id| {
        mask | NODE_TOPOLOGY.get(id).expect("Id must not exceed max cpus")
    }))
}

#[inline]
pub fn smt_affinity(id: Id) -> impl Iterator<Item = Id> {
    from_mask(*SMT_TOPOLOGY.get(id).expect("ID must not exceed max cpus"))
}

#[inline]
pub fn node_affinity(id: Id) -> impl Iterator<Item = Id> {
    from_mask(*NODE_TOPOLOGY.get(id).expect("ID must not exceed max cpus"))
}

#[inline]
#[must_use]
pub fn available_parallelism() -> usize {
    cores_available().count()
}

#[inline]
#[must_use]
pub fn nth_core_available(i: usize) -> Option<Id> {
    cores_available().nth(i)
}

#[inline]
#[must_use]
pub fn is_core_available(id: Id) -> bool {
    cores_available().any(is_equal_to!(id))
}

#[inline]
pub fn cores_available() -> impl Iterator<Item = Id> {
    from_mask(*CORES_AVAILABLE)
}

#[cfg(target_os = "linux")]
#[inline]
#[expect(unsafe_code, reason = "calling getcpu(2)")]
pub fn getcpu() -> Result<usize> {
    use crate::{Error, math};

    let ret: i32 = unsafe { libc::sched_getcpu() };

    if ret < 0 {
        return Err(Error::from_errno());
    }

    math::try_into(ret)
}

#[cfg(not(target_os = "linux"))]
#[inline]
pub fn getcpu() -> Result<usize> {
    Err(crate::Error::Io(std::io::ErrorKind::Unsupported.into()))
}

#[cfg(any(target_os = "linux", target_os = "android"))]
#[expect(unsafe_code, reason = "building a cpu_set_t for sched_setaffinity(2)")]
fn set_each_for_current<I>(ids: I) -> bool
where
    I: Iterator<Item = Id>,
{
    let mut set: libc::cpu_set_t = unsafe { std::mem::zeroed() };

    for id in ids {
        debug_assert!(
            id < libc::CPU_SETSIZE as usize,
            "core ID must be < CPU_SETSIZE"
        );

        unsafe { libc::CPU_SET(id, &mut set) };
    }

    let ret = unsafe { libc::sched_setaffinity(0, size_of::<libc::cpu_set_t>(), &set) };

    debug_assert!(
        ret == 0,
        "sched_setaffinity() failed: {}",
        std::io::Error::last_os_error()
    );

    ret == 0
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn set_each_for_current<I>(ids: I) -> bool
where
    I: Iterator<Item = Id>,
{
    ids.map(|id| core_affinity::set_for_current(core_affinity::CoreId { id }))
        .fold(true, |ok, res| ok && res)
}

fn query_cores_available() -> impl Iterator<Item = Id> {
    core_affinity::get_core_ids()
        .unwrap_or_default()
        .into_iter()
        .map(|core_id| core_id.id)
}

fn init_smt_topology() -> [Mask; MASK_BITS] {
    [Mask::default(); MASK_BITS]
}

fn init_node_topology() -> [Mask; MASK_BITS] {
    [Mask::default(); MASK_BITS]
}

fn into_mask<I>(ids: I) -> Mask
where
    I: Iterator<Item = Id>,
{
    ids.inspect(|&id| {
        debug_assert!(
            id < MASK_BITS,
            "Core ID must be < Mask::BITS at least for now"
        );
    })
    .fold(Mask::default(), |mask, id| mask | (1 << id))
}

fn from_mask(v: Mask) -> impl Iterator<Item = Id> {
    (0..MASK_BITS).filter(move |&i| (v & (1 << i)) != 0)
}

fn _sys_path(id: usize, suffix: &str) -> PathBuf {
    format!("/sys/devices/system/cpu/cpu{id}/{suffix}").into()
}
