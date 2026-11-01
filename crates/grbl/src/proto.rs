//! GRBL 1.1 lines: what the machine says ([`parse_line`]) and what we may send ([`clean_line`]).
//!
//! Written from GRBL's public "Grbl v1.1 Interface" documentation. Every parser takes any text and
//! never panics: anything it doesn't recognise comes back as [`Response::Other`].

/// Realtime command: ask for a status report (no newline, not counted in the receive buffer).
pub const STATUS: u8 = b'?';
/// Realtime command: feed hold (decelerate to a stop, keep the job).
pub const HOLD: u8 = b'!';
/// Realtime command: cycle start / resume after a hold.
pub const RESUME: u8 = b'~';
/// Realtime command: soft reset (ctrl-X): stops at once, purges the buffers, turns the laser off.
pub const RESET: u8 = 0x18;
/// Realtime command: cancel the current jog and flush queued jog moves.
pub const JOG_CANCEL: u8 = 0x85;

/// GRBL's serial receive buffer (bytes) on the classic Arduino builds; `[OPT:…]` may report less.
pub const RX_BUFFER: usize = 128;
/// The longest G-code line GRBL accepts (its line buffer; longer lines get `error:11`).
pub const MAX_LINE: usize = 80;
/// The longest incoming line kept; longer ones are cut (GRBL's own lines are far shorter).
pub const MAX_INCOMING: usize = 256;
/// Cap on the pin and accessory letter strings of a status report.
const MAX_LETTERS: usize = 16;

/// The machine's state, the first field of a status report.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MachineState {
    Idle,
    Run,
    Hold,
    Jog,
    Alarm,
    Door,
    Check,
    Home,
    Sleep,
    /// Not reported yet, or a state this version doesn't know.
    #[default]
    Unknown,
}

impl MachineState {
    /// The state as GRBL writes it (`"Idle"`, `"Run"`, …; `"Unknown"`).
    pub fn as_str(self) -> &'static str {
        match self {
            MachineState::Idle => "Idle",
            MachineState::Run => "Run",
            MachineState::Hold => "Hold",
            MachineState::Jog => "Jog",
            MachineState::Alarm => "Alarm",
            MachineState::Door => "Door",
            MachineState::Check => "Check",
            MachineState::Home => "Home",
            MachineState::Sleep => "Sleep",
            MachineState::Unknown => "Unknown",
        }
    }

    /// Parse GRBL's name of a state (exact, case-sensitive); anything else is `Unknown`.
    pub fn parse(s: &str) -> MachineState {
        match s {
            "Idle" => MachineState::Idle,
            "Run" => MachineState::Run,
            "Hold" => MachineState::Hold,
            "Jog" => MachineState::Jog,
            "Alarm" => MachineState::Alarm,
            "Door" => MachineState::Door,
            "Check" => MachineState::Check,
            "Home" => MachineState::Home,
            "Sleep" => MachineState::Sleep,
            _ => MachineState::Unknown,
        }
    }
}

/// A status report (`<Idle|MPos:0.000,0.000,0.000|FS:0,0>`). GRBL sends either the machine
/// position or the work position (setting `$10`); with the work offset (`WCO`) one gives the other
/// (`WPos = MPos − WCO`). The [`crate::Controller`] fills in the missing one when it knows `WCO`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub state: MachineState,
    /// Substate: `Hold:0` (hold complete) / `Hold:1` (stopping), `Door:0..3`.
    pub sub: Option<u8>,
    /// Machine position (mm), when reported (or derived).
    pub mpos: Option<[f64; 3]>,
    /// Work position (mm), when reported (or derived).
    pub wpos: Option<[f64; 3]>,
    /// Work coordinate offset (mm); GRBL sends it now and then, not in every report.
    pub wco: Option<[f64; 3]>,
    /// Current feed rate (mm/min).
    pub feed: Option<f64>,
    /// Current spindle speed, i.e. laser power in S units.
    pub spindle: Option<f64>,
    /// `Bf:` free planner blocks and free receive-buffer bytes.
    pub buffer: Option<(u32, u32)>,
    /// `Ln:` the line number being executed (only for lines with `N` words).
    pub line: Option<u32>,
    /// `Ov:` feed, rapid and spindle overrides (percent).
    pub ov: Option<[u32; 3]>,
    /// `Pn:` input pins triggered (`X Y Z P D H R S`), empty when none.
    pub pins: String,
    /// `A:` accessories (`S` spindle CW, `C` CCW, `F` flood, `M` mist), empty when none reported.
    pub accessories: String,
}

/// One line from the machine.
#[derive(Clone, Debug, PartialEq)]
pub enum Response {
    /// `ok`: the oldest line sent was accepted.
    Ok,
    /// `error:N`: the oldest line sent was rejected (see [`crate::error_text`]).
    Error(u16),
    /// `ALARM:N`: the machine is in alarm (see [`crate::alarm_text`]).
    Alarm(u16),
    /// `<…>` status report.
    Status(Status),
    /// The welcome banner `Grbl 1.1h ['$' for help]`, after power-up or a reset.
    Welcome { version: String },
    /// `$n=value` from `$$`.
    Setting(u16, String),
    /// `[MSG:…]` feedback message.
    Message(String),
    /// Other `[…]` feedback (`VER`, `OPT`, `GC`, `HLP`, `echo`, parameters), brackets removed.
    Feedback(String),
    /// Anything else (startup line results `>…:ok`, noise).
    Other(String),
}

/// Parse one line from the machine (line ending optional). Never panics.
pub fn parse_line(line: &str) -> Response {
    let s = cap_str(line.trim(), MAX_INCOMING);
    if s == "ok" {
        return Response::Ok;
    }
    if let Some(n) = s.strip_prefix("error:") {
        return n.trim().parse().map(Response::Error).unwrap_or_else(|_| Response::Other(s.to_string()));
    }
    if let Some(n) = s.strip_prefix("ALARM:") {
        return n.trim().parse().map(Response::Alarm).unwrap_or_else(|_| Response::Other(s.to_string()));
    }
    if let Some(inner) = s.strip_prefix('<').and_then(|r| r.strip_suffix('>')) {
        return parse_status(inner).map(Response::Status).unwrap_or_else(|| Response::Other(s.to_string()));
    }
    if let Some(version) = welcome_version(s) {
        return Response::Welcome { version };
    }
    if let Some(inner) = s.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
        return match inner.strip_prefix("MSG:") {
            Some(m) => Response::Message(m.to_string()),
            None => Response::Feedback(inner.to_string()),
        };
    }
    if let Some((n, v)) = s.strip_prefix('$').and_then(|r| r.split_once('='))
        && let Ok(n) = n.parse::<u16>()
    {
        return Response::Setting(n, v.trim().to_string());
    }
    Response::Other(s.to_string())
}

/// `Grbl 1.1h ['$' for help]` → `1.1h`. The banner may follow line noise after a reset, so it is
/// looked for anywhere in the line.
fn welcome_version(s: &str) -> Option<String> {
    let at = s.find("Grbl ")?;
    let rest = s.get(at + 5..)?;
    if !rest.contains("for help]") {
        return None;
    }
    let version = rest.split_whitespace().next()?;
    Some(cap_str(version, 32).to_string())
}

/// Parse the inside of `<…>`. `None` when the state field is missing.
pub fn parse_status(inner: &str) -> Option<Status> {
    let mut fields = inner.split('|');
    let head = fields.next()?;
    if head.is_empty() {
        return None;
    }
    let (state, sub) = match head.split_once(':') {
        Some((st, sub)) => (st, sub.parse::<u8>().ok()),
        None => (head, None),
    };
    let mut out = Status { state: MachineState::parse(state), sub, ..Status::default() };
    for f in fields {
        status_field(&mut out, f);
    }
    Some(out)
}

fn status_field(out: &mut Status, f: &str) {
    let Some((key, val)) = f.split_once(':') else { return };
    match key {
        "MPos" => out.mpos = axes(val),
        "WPos" => out.wpos = axes(val),
        "WCO" => out.wco = axes(val),
        "Bf" => {
            let v = numbers::<2>(val);
            out.buffer = v.map(|[a, b]| (to_u32(a), to_u32(b)));
        }
        "Ln" => out.line = val.parse().ok(),
        "FS" => {
            let mut it = val.split(',');
            out.feed = it.next().and_then(finite);
            out.spindle = it.next().and_then(finite);
        }
        "F" => out.feed = finite(val),
        "Ov" => out.ov = numbers::<3>(val).map(|v| v.map(to_u32)),
        "Pn" => out.pins = letters(val),
        "A" => out.accessories = letters(val),
        _ => {}
    }
}

/// Up to the first three of 1–6 comma-separated axes, missing ones 0; `None` when any is not a
/// finite number.
fn axes(val: &str) -> Option<[f64; 3]> {
    let mut out = [0.0; 3];
    let mut n = 0;
    for (i, part) in val.split(',').enumerate() {
        let v = finite(part)?;
        if let Some(slot) = out.get_mut(i) {
            *slot = v;
        }
        n = i + 1;
        if n > 6 {
            return None;
        }
    }
    (n > 0).then_some(out)
}

fn numbers<const N: usize>(val: &str) -> Option<[f64; N]> {
    let mut out = [0.0; N];
    let mut it = val.split(',');
    for slot in out.iter_mut() {
        *slot = finite(it.next()?)?;
    }
    Some(out)
}

fn finite(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.len() > 24 {
        return None;
    }
    s.parse::<f64>().ok().filter(|v| v.is_finite())
}

fn to_u32(v: f64) -> u32 {
    if v <= 0.0 {
        0
    } else if v >= u32::MAX as f64 {
        u32::MAX
    } else {
        v as u32
    }
}

fn letters(val: &str) -> String {
    val.chars().filter(|c| c.is_ascii_alphabetic()).take(MAX_LETTERS).collect()
}

/// The longest prefix of `s` of at most `max` bytes that ends on a char boundary.
pub fn cap_str(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.get(..end).unwrap_or("")
}

/// Make a G-code line safe to stream: comments (`;` to the end, `( … )`) and all whitespace
/// removed. Refused (with a reason): non-ASCII or control characters, an unclosed `(`, the
/// realtime characters `?`, `!`, `~` (GRBL would act on them at once instead of in order), and
/// anything over [`MAX_LINE`] bytes once cleaned. An empty result means "nothing to send".
pub fn clean_line(line: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut in_paren = false;
    for c in line.chars() {
        if in_paren {
            if c == ')' {
                in_paren = false;
            }
            continue;
        }
        match c {
            ';' => break,
            '(' => in_paren = true,
            c if c.is_whitespace() => {}
            '?' | '!' | '~' => return Err(format!("contains '{c}', which GRBL reads as a realtime command")),
            c if !c.is_ascii() => return Err("contains a character that isn't plain ASCII".into()),
            c if c.is_ascii_control() => return Err("contains a control character".into()),
            c => {
                out.push(c);
                if out.len() > MAX_LINE {
                    return Err(format!("is longer than GRBL's {MAX_LINE} characters"));
                }
            }
        }
    }
    if in_paren {
        return Err("has a '(' comment that is never closed".into());
    }
    Ok(out)
}
