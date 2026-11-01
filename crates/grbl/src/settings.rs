//! The machine's `$$` settings (GRBL 1.1 numbers every setting; all of them are numeric).

use std::collections::BTreeMap;

/// The most settings kept (GRBL 1.1 has about 35).
const MAX_SETTINGS: usize = 256;

/// The settings read with `$$`, by number.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Settings {
    pub values: BTreeMap<u16, f64>,
}

impl Settings {
    /// Record `$n=value`. Returns false (and keeps nothing) when the value isn't a finite number
    /// or the map is full.
    pub fn set(&mut self, n: u16, value: &str) -> bool {
        let Ok(v) = value.trim().parse::<f64>() else { return false };
        if !v.is_finite() || (self.values.len() >= MAX_SETTINGS && !self.values.contains_key(&n)) {
            return false;
        }
        self.values.insert(n, v);
        true
    }

    /// Setting `$n`.
    pub fn get(&self, n: u16) -> Option<f64> {
        self.values.get(&n).copied()
    }

    /// `$32`: laser mode on (must be on to cut).
    pub fn laser_mode(&self) -> Option<bool> {
        self.get(32).map(|v| v != 0.0)
    }

    /// `$30`: the S value of full power.
    pub fn s_max(&self) -> Option<f64> {
        self.get(30)
    }

    /// `$130`–`$132`: maximum travel of X, Y and Z (mm), when all three are known.
    pub fn max_travel(&self) -> Option<[f64; 3]> {
        Some([self.get(130)?, self.get(131)?, self.get(132)?])
    }

    /// `$22`: homing cycle enabled.
    pub fn homing(&self) -> Option<bool> {
        self.get(22).map(|v| v != 0.0)
    }

    /// `$20`: soft limits enabled.
    pub fn soft_limits(&self) -> Option<bool> {
        self.get(20).map(|v| v != 0.0)
    }

    /// `$10` bit 0: status reports give the machine position (true) or the work position (false).
    pub fn report_mpos(&self) -> Option<bool> {
        self.get(10).map(|v| (0.0..=255.0).contains(&v) && (v as u32) & 1 == 1)
    }
}
