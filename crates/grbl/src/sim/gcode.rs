//! The simulated machine's interpreter: G-code lines, `$` commands, jogs.

use super::{Block, HOMING_MS, MoveKind, PLANNER_BLOCKS, SimGrbl, Spindle, num};
use crate::MachineState;

impl SimGrbl {
    /// Spindle changes, dwells, `$` commands and G10 wait for the planner to empty.
    pub(super) fn needs_sync(&self, line: &str) -> bool {
        if line.starts_with('$') {
            return !line.starts_with("$J=");
        }
        let Ok(words) = words(line) else { return false };
        words.iter().any(|&(c, v)| (c == 'M' && matches!(v as i64, 2..=5 | 30)) || (c == 'G' && matches!(v as i64, 4 | 10)))
    }

    pub(super) fn line(&mut self, line: &str) {
        if line.is_empty() {
            self.ok();
        } else if let Some(cmd) = line.strip_prefix('$') {
            self.dollar(cmd);
        } else {
            self.stats.gcode_lines += 1;
            let n = self.stats.gcode_lines;
            if self.opt.alarm_at_line == Some(n) {
                self.raise_alarm(1);
            } else if self.opt.error_at_line == Some(n) {
                self.error(20);
            } else if self.state == MachineState::Alarm || self.state == MachineState::Jog {
                self.error(9);
            } else {
                match self.gcode(line) {
                    Ok(()) => {
                        if self.dwell_until.is_none() {
                            self.ok();
                        }
                    }
                    Err(code) => self.error(code),
                }
            }
        }
    }

    fn dollar(&mut self, cmd: &str) {
        let idle = matches!(self.state, MachineState::Idle | MachineState::Alarm);
        if let Some(jog) = cmd.strip_prefix("J=") {
            return match self.jog(jog) {
                Ok(()) => self.ok(),
                Err(e) => self.error(e),
            };
        }
        if !idle {
            return self.error(8);
        }
        match cmd {
            "$" => {
                let lines: Vec<String> = self.settings.iter().map(|(n, v)| format!("${n}={}", num(*v))).collect();
                for l in lines {
                    self.emit(&l);
                }
                self.ok();
            }
            "I" => {
                self.emit("[VER:1.1h.SIM:]");
                self.emit(&format!("[OPT:V,{PLANNER_BLOCKS},{}]", self.opt.rx_size));
                self.ok();
            }
            "H" => {
                if !self.opt.homing {
                    return self.error(5);
                }
                self.state = MachineState::Home;
                self.home_until = Some(self.t + HOMING_MS / self.opt.speed_factor.max(1e-6));
            }
            "X" => {
                if self.state == MachineState::Alarm {
                    self.state = MachineState::Idle;
                    self.alarm = None;
                    self.critical = false;
                    self.emit("[MSG:Caution: Unlocked]");
                }
                self.ok();
            }
            "G" => {
                let g = format!(
                    "[GC:G{} G54 G17 {} {} G94 {} M9 T0 F{} S{}]",
                    self.modal.motion,
                    if self.modal.inches { "G20" } else { "G21" },
                    if self.modal.absolute { "G90" } else { "G91" },
                    match self.modal.spindle {
                        Spindle::Off => "M5",
                        Spindle::Cw => "M3",
                        Spindle::Ccw => "M4",
                    },
                    num(self.modal.feed),
                    num(self.modal.s)
                );
                self.emit(&g);
                self.ok();
            }
            "#" => {
                self.emit(&format!("[G54:{},{},{}]", num(self.wco[0]), num(self.wco[1]), num(self.wco[2])));
                self.ok();
            }
            "N" => {
                self.emit("$N0=");
                self.emit("$N1=");
                self.ok();
            }
            _ => self.setting_write(cmd),
        }
    }

    fn setting_write(&mut self, cmd: &str) {
        let Some((n, v)) = cmd.split_once('=') else { return self.error(3) };
        let (Ok(n), Ok(v)) = (n.parse::<u16>(), v.parse::<f64>()) else { return self.error(2) };
        if !v.is_finite() || !self.settings.contains_key(&n) {
            return self.error(3);
        }
        self.settings.insert(n, v);
        match n {
            32 => self.opt.laser_mode = v != 0.0,
            30 => self.opt.s_max = v,
            22 => self.opt.homing = v != 0.0,
            _ => {}
        }
        self.ok();
    }

    fn gcode(&mut self, line: &str) -> Result<(), u16> {
        let words = words(line)?;
        let mut m = self.modal;
        let mut axes: [Option<f64>; 3] = [None; 3];
        let mut motion = None;
        let (mut dwell, mut g10, mut p, mut l) = (false, false, None, None);
        for &(c, v) in &words {
            match c {
                'G' => match g_code(v)? {
                    c @ (0 | 10 | 20 | 30) => motion = Some((c / 10) as u8),
                    40 => dwell = true,
                    100 => g10 = true,
                    170 | 540 | 940 => {}
                    200 => m.inches = true,
                    210 => m.inches = false,
                    900 => m.absolute = true,
                    910 => m.absolute = false,
                    _ => return Err(20),
                },
                'M' => match v as i64 {
                    3 if v == 3.0 => m.spindle = Spindle::Cw,
                    4 if v == 4.0 => m.spindle = Spindle::Ccw,
                    5 | 2 | 30 if v.fract() == 0.0 => m.spindle = Spindle::Off,
                    8 | 9 if v.fract() == 0.0 => {}
                    _ => return Err(20),
                },
                'X' => axes[0] = Some(v),
                'Y' => axes[1] = Some(v),
                'Z' => axes[2] = Some(v),
                'F' => m.feed = if v < 0.0 { return Err(4) } else { v },
                'S' => m.s = if v < 0.0 { return Err(4) } else { v.min(self.opt.s_max) },
                'P' => p = Some(v),
                'L' => l = Some(v),
                'N' | 'I' | 'J' | 'K' | 'R' => {}
                _ => return Err(20),
            }
        }
        let scale = if m.inches { 25.4 } else { 1.0 };
        if g10 {
            if l != Some(20.0) || p.is_none() {
                return Err(28);
            }
            for (i, a) in axes.iter().enumerate() {
                if let (Some(v), Some(w), Some(pos)) = (a, self.wco.get_mut(i), self.pos.get(i)) {
                    *w = pos - v * scale;
                }
            }
            self.modal = m;
            return Ok(());
        }
        if dwell {
            let secs = p.ok_or(28u16)?;
            self.modal = m;
            self.dwell_until = Some(self.t + (secs.max(0.0) * 1000.0));
            return Ok(());
        }
        if let Some(mo) = motion {
            m.motion = mo;
        }
        let has_axes = axes.iter().any(Option::is_some);
        if has_axes && m.motion != 0 && m.feed <= 0.0 {
            return Err(22);
        }
        self.modal = m;
        if has_axes {
            let target = self.target(&axes, scale, m.absolute);
            let kind = if m.motion == 0 { MoveKind::Rapid } else { MoveKind::Feed };
            let feed = if kind == MoveKind::Rapid { self.opt.rapid_mm_min } else { m.feed * scale };
            self.queue(Block { kind, target, feed, spindle: m.spindle, s: m.s, cutting_mode: m.motion != 0 });
        }
        Ok(())
    }

    fn target(&self, axes: &[Option<f64>; 3], scale: f64, absolute: bool) -> [f64; 3] {
        let base = self.planner.back().map_or(self.pos, |b| b.target);
        let mut t = base;
        for i in 0..3 {
            if let (Some(Some(v)), Some(slot), Some(w)) = (axes.get(i), t.get_mut(i), self.wco.get(i)) {
                *slot = if absolute { v * scale + w } else { *slot + v * scale };
            }
        }
        t
    }

    fn queue(&mut self, b: Block) {
        self.planner.push_back(b);
        if self.state == MachineState::Idle {
            self.state = if b.kind == MoveKind::Jog { MachineState::Jog } else { MachineState::Run };
        }
    }

    fn jog(&mut self, jog: &str) -> Result<(), u16> {
        if !matches!(self.state, MachineState::Idle | MachineState::Jog) {
            return Err(if self.state == MachineState::Alarm { 9 } else { 8 });
        }
        let words = words(jog)?;
        let mut axes: [Option<f64>; 3] = [None; 3];
        let (mut absolute, mut inches, mut feed) = (true, false, None);
        for &(c, v) in &words {
            match c {
                'G' if v == 90.0 => absolute = true,
                'G' if v == 91.0 => absolute = false,
                'G' if v == 20.0 => inches = true,
                'G' if v == 21.0 || v == 53.0 => {}
                'X' => axes[0] = Some(v),
                'Y' => axes[1] = Some(v),
                'Z' => axes[2] = Some(v),
                'F' => feed = Some(v),
                _ => return Err(16),
            }
        }
        let feed = feed.filter(|f| *f > 0.0).ok_or(16u16)?;
        if axes.iter().all(Option::is_none) {
            return Err(16);
        }
        let scale = if inches { 25.4 } else { 1.0 };
        let target = self.target(&axes, scale, absolute);
        for (i, v) in target.iter().enumerate() {
            let max = self.opt.max_travel.get(i).copied().unwrap_or(0.0);
            if self.settings.get(&20).is_some_and(|s| *s != 0.0) && !(0.0..=max).contains(v) {
                return Err(15);
            }
        }
        let m = self.modal;
        self.queue(Block { kind: MoveKind::Jog, target, feed: feed * scale, spindle: m.spindle, s: m.s, cutting_mode: (1..=3).contains(&m.motion) });
        Ok(())
    }
}

/// A G number × 10 (G17 → 170, G28.1 → 281); `Err(20)` for anything finer or out of range.
fn g_code(v: f64) -> Result<i64, u16> {
    let c = (v * 10.0).round();
    if !(0.0..=10_000.0).contains(&c) || (c - v * 10.0).abs() > 1e-6 {
        return Err(20);
    }
    Ok(c as i64)
}

/// A G-code line's words (letter, value). `Err(1)` for a value without a letter, `Err(2)` for a
/// letter without a number.
fn words(line: &str) -> Result<Vec<(char, f64)>, u16> {
    let mut out = Vec::new();
    let mut chars = line.chars().filter(|c| !c.is_whitespace()).peekable();
    while let Some(c) = chars.next() {
        if !c.is_ascii_alphabetic() {
            return Err(1);
        }
        let mut num = String::new();
        while let Some(&d) = chars.peek() {
            if d.is_ascii_digit() || d == '.' || d == '-' || d == '+' {
                num.push(d);
                chars.next();
            } else {
                break;
            }
        }
        let v: f64 = num.parse().map_err(|_| 2u16)?;
        if !v.is_finite() {
            return Err(2);
        }
        out.push((c.to_ascii_uppercase(), v));
    }
    Ok(out)
}
