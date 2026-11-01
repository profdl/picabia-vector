//! Where the controller's bytes go: a serial port natively ([`crate::serial`]), the simulator in
//! tests ([`crate::sim::SimTransport`]), later WebSerial.

use std::io;

use crate::Controller;

/// A byte pipe to a GRBL machine.
pub trait Transport: Send {
    /// Write all of `data`.
    fn write(&mut self, data: &[u8]) -> io::Result<()>;
    /// Read what has arrived, waiting at most a short timeout; `Ok(0)` means nothing arrived.
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize>;
    /// A name for the status line (the port path, "Simulated GRBL").
    fn name(&self) -> &str;
}

/// The most bytes [`pump`] reads in one call before polling (keeps one step short).
const MAX_READ_PER_PUMP: usize = 16 * 1024;

/// One step of the I/O loop: read what arrived into the controller, then write what it wants to
/// send. An I/O error is returned as is (the caller calls [`Controller::on_disconnect`]).
pub fn pump(ctrl: &mut Controller, transport: &mut dyn Transport, now_ms: u64) -> io::Result<()> {
    let mut buf = [0u8; 1024];
    let mut total = 0;
    while total < MAX_READ_PER_PUMP {
        let n = transport.read(&mut buf)?;
        let Some(chunk) = buf.get(..n) else { break };
        if chunk.is_empty() {
            break;
        }
        ctrl.on_bytes(chunk, now_ms);
        total += n;
    }
    let out = ctrl.poll(now_ms);
    if !out.is_empty() {
        transport.write(&out)?;
    }
    Ok(())
}
