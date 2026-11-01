//! The crate's deterministic random numbers: SplitMix64 streams, no `rand`, no platform state.
//! Integer arithmetic plus one exact `u64 -> f64` scaling, so values are identical on every
//! platform and in wasm.

use crate::eval::splitmix64;

/// A stream of numbers fixed by `(seed, index)`.
pub(crate) struct Stream(u64);

impl Stream {
    pub fn new(seed: u32, index: usize) -> Stream {
        let idx = u64::try_from(index).unwrap_or(u64::MAX);
        Stream(splitmix64((u64::from(seed) << 32) ^ idx.wrapping_mul(0xD6E8_FEB8_6659_FD93)))
    }

    /// The next number in `-1.0..1.0`.
    pub fn signed(&mut self) -> f64 {
        self.0 = splitmix64(self.0);
        // 53 random bits to a double in [0, 1): exact.
        let unit = (self.0 >> 11) as f64 / 9_007_199_254_740_992.0;
        unit * 2.0 - 1.0
    }
}
