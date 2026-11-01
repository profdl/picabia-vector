//! GRBL 1.1 error and alarm codes in plain English, written for Picabia from the meanings in GRBL's
//! public documentation tables. The advice is Picabia's own.

/// What `error:N` means. An `error` answers one line: that line was not run.
pub fn error_text(code: u16) -> &'static str {
    match code {
        1 => "A word in the line has no letter (G-code words are a letter and a number).",
        2 => "A number is missing or badly written.",
        3 => "The machine doesn't know this '$' command.",
        4 => "A value that must be positive is negative.",
        5 => "Homing isn't turned on in the machine's settings ($22).",
        6 => "The step pulse time must be at least 3 microseconds.",
        7 => "The machine couldn't read its stored settings and went back to its defaults: check them.",
        8 => "This '$' command only works when the machine is idle.",
        9 => "G-code is locked while the machine is in alarm or jogging: home or unlock it first.",
        10 => "Soft limits need homing to be turned on.",
        11 => "The line is longer than the machine accepts; it was not run.",
        12 => "This setting would make the motors step faster than the machine can.",
        13 => "The safety door is open.",
        14 => "The text is too long to store in the machine's memory.",
        15 => "The jog would go past the machine's travel; it was ignored.",
        16 => "The jog command is malformed or contains G-code a jog can't use.",
        17 => "Laser mode needs the machine's PWM output.",
        20 => "The line has a G-code command the machine doesn't support.",
        21 => "The line has two commands from the same group (for example G0 and G1).",
        22 => "No feed rate has been set (an F word is needed).",
        23 => "A command in the line needs a whole number.",
        24 => "Two commands in the line both want the axis words.",
        25 => "A word appears twice in the line.",
        26 => "The command needs axis words (X, Y or Z) and the line has none.",
        27 => "The line number (N) is not valid.",
        28 => "The command is missing a value it needs.",
        29 => "Work coordinate systems G59.1 and up aren't supported.",
        30 => "G53 only works with G0 and G1.",
        31 => "The line has axis words that nothing in it uses.",
        32 => "An arc (G2/G3) needs at least one axis word in its plane.",
        33 => "The move's target is not valid.",
        34 => "The arc's radius is not valid.",
        35 => "An arc (G2/G3) needs at least one offset word in its plane.",
        36 => "The line has value words that nothing in it uses.",
        37 => "The tool length offset is not on the tool length axis.",
        38 => "The tool number is larger than the machine supports.",
        _ => "The machine reported an error this version of Picabia doesn't know.",
    }
}

/// What `ALARM:N` means.
pub fn alarm_text(code: u16) -> &'static str {
    match code {
        1 => "Hard limit: a limit switch was hit while moving.",
        2 => "Soft limit: the job would move past the machine's travel.",
        3 => "Reset while moving: the machine was reset during a motion.",
        4 => "Probe fail: the probe was already in the wrong state before probing.",
        5 => "Probe fail: the probe never touched within the probing distance.",
        6 => "Homing fail: homing was reset before it finished.",
        7 => "Homing fail: the safety door was opened during homing.",
        8 => "Homing fail: backing off didn't release the limit switch.",
        9 => "Homing fail: no limit switch was found within the search distance.",
        10 => "Homing fail: the second switch of a dual axis wasn't found.",
        _ => "The machine raised an alarm this version of Picabia doesn't know.",
    }
}

/// What to do about `ALARM:N`, in plain words.
pub fn alarm_advice(code: u16) -> &'static str {
    match code {
        1 => {
            "The position is probably lost. Check nothing is in the way and the switches aren't triggered, reset, then home the machine before cutting again."
        }
        2 => "The position is still known. Move the job (or the origin) so it fits the bed, then unlock and start again.",
        3 => "The position may be lost after a sudden stop. Home the machine (or unlock and set the origin again) before the next job.",
        4 | 5 => "Check the probe and its wiring, then unlock and try again.",
        6 => "Start homing again and let it finish.",
        7 => "Close the door, then start homing again.",
        8 => "Check the limit switch wiring, or increase the homing pull-off ($27), then home again.",
        9 => "Check the limit switch wiring, or increase the maximum travel ($130-$132), then home again.",
        10 => "Check the second switch's wiring, then home again.",
        _ => "Check the machine, then unlock it ($X) or home it before continuing.",
    }
}
