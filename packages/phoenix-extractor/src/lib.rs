mod registry;
mod xyk;

pub use registry::{PhoenixPool, PhoenixPoolRegistry};
pub use xyk::PhoenixXykExtractor;

pub const PHOENIX_XYK_EVENT_COUNT: usize = 8;
pub const PHOENIX_STABLE_EVENT_COUNT: usize = 6;

pub const POOL_TYPE_XYK: u32 = 0;
