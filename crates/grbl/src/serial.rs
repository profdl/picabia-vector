//! The native serial transport (not in the browser build): the ports, and a [`Transport`] over
//! one at 8N1 with DTR set (many GRBL boards reset when DTR rises; the controller waits for the
//! banner either way). Uses the `serialport` crate (MPL-2.0, unmodified).

use std::io::{self, Read, Write};
use std::time::Duration;

use serialport::{DataBits, FlowControl, Parity, SerialPort, SerialPortType, StopBits};

use crate::Transport;

/// GRBL 1.1's default baud rate.
pub const DEFAULT_BAUD: u32 = 115_200;
/// How long a read waits for bytes.
pub const READ_TIMEOUT_MS: u64 = 20;
/// The most ports listed.
const MAX_PORTS: usize = 64;

/// A serial port, for the port picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortInfo {
    /// What to pass to [`open`] (`/dev/cu.usbserial-1410`, `COM3`).
    pub path: String,
    /// A name a person recognises: the USB product and maker when known, else the path.
    pub name: String,
    /// The port is a USB device (lasers are; Bluetooth and built-in ports usually aren't).
    pub usb: bool,
}

/// The serial ports, USB ones first. On macOS only the `/dev/cu.*` call-out devices are listed
/// (opening a `/dev/tty.*` waits for a carrier signal a laser never raises). An error listing
/// ports gives an empty list.
pub fn list_ports() -> Vec<PortInfo> {
    let Ok(ports) = serialport::available_ports() else { return Vec::new() };
    let mut out: Vec<PortInfo> = ports
        .into_iter()
        .filter(|p| !(cfg!(target_os = "macos") && p.port_name.starts_with("/dev/tty.")))
        .map(|p| {
            let (name, usb) = friendly_name(&p.port_name, &p.port_type);
            PortInfo { path: p.port_name, name, usb }
        })
        .take(MAX_PORTS)
        .collect();
    out.sort_by(|a, b| b.usb.cmp(&a.usb).then_with(|| a.path.cmp(&b.path)));
    out
}

fn friendly_name(path: &str, kind: &SerialPortType) -> (String, bool) {
    let short = path.rsplit('/').next().unwrap_or(path).to_string();
    match kind {
        SerialPortType::UsbPort(usb) => {
            let product = usb.product.as_deref().map(str::trim).filter(|s| !s.is_empty());
            let maker = usb.manufacturer.as_deref().map(str::trim).filter(|s| !s.is_empty());
            let label = match (product, maker) {
                (Some(p), Some(m)) if !p.contains(m) => format!("{p} ({m})"),
                (Some(p), _) => p.to_string(),
                (None, Some(m)) => format!("{m} USB serial"),
                (None, None) => format!("USB serial {:04x}:{:04x}", usb.vid, usb.pid),
            };
            (format!("{} - {short}", crate::proto::cap_str(&label, 80)), true)
        }
        SerialPortType::BluetoothPort => (format!("Bluetooth - {short}"), false),
        SerialPortType::PciPort | SerialPortType::Unknown => (short, false),
    }
}

/// An open serial port.
pub struct SerialTransport {
    port: Box<dyn SerialPort>,
    name: String,
}

/// Open `path` at `baud` (0 = [`DEFAULT_BAUD`]), 8 data bits, no parity, 1 stop bit, no flow
/// control, a [`READ_TIMEOUT_MS`] read timeout, DTR set.
pub fn open(path: &str, baud: u32) -> io::Result<SerialTransport> {
    let baud = if baud == 0 { DEFAULT_BAUD } else { baud };
    let mut port = serialport::new(path, baud)
        .data_bits(DataBits::Eight)
        .parity(Parity::None)
        .stop_bits(StopBits::One)
        .flow_control(FlowControl::None)
        .timeout(Duration::from_millis(READ_TIMEOUT_MS))
        .open()
        .map_err(io::Error::from)?;
    port.write_data_terminal_ready(true).map_err(io::Error::from)?;
    Ok(SerialTransport { port, name: path.to_string() })
}

impl Transport for SerialTransport {
    fn write(&mut self, data: &[u8]) -> io::Result<()> {
        self.port.write_all(data)?;
        self.port.flush()
    }

    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.port.read(buf) {
            Ok(n) => Ok(n),
            Err(e) if matches!(e.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted) => Ok(0),
            Err(e) => Err(e),
        }
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn friendly_names() {
        let usb = |product: Option<&str>, maker: Option<&str>| {
            SerialPortType::UsbPort(serialport::UsbPortInfo {
                vid: 0x2341,
                pid: 0x0043,
                serial_number: None,
                manufacturer: maker.map(str::to_string),
                product: product.map(str::to_string),
            })
        };
        assert_eq!(
            friendly_name("/dev/cu.usbmodem1411", &usb(Some("Arduino Uno"), Some("Arduino (www.arduino.cc)"))),
            ("Arduino Uno (Arduino (www.arduino.cc)) - cu.usbmodem1411".to_string(), true)
        );
        assert_eq!(friendly_name("/dev/cu.x", &usb(Some("FT232R USB UART"), Some("FTDI"))).0, "FT232R USB UART (FTDI) - cu.x");
        assert_eq!(friendly_name("/dev/cu.y", &usb(Some("Arduino Uno"), Some("Arduino"))).0, "Arduino Uno - cu.y");
        assert_eq!(friendly_name("COM3", &usb(None, None)).0, "USB serial 2341:0043 - COM3");
        assert_eq!(friendly_name("COM4", &usb(None, Some("wch.cn"))).0, "wch.cn USB serial - COM4");
        assert_eq!(friendly_name("/dev/ttyS0", &SerialPortType::PciPort), ("ttyS0".to_string(), false));
    }

    #[test]
    fn opening_a_missing_port_is_an_error() {
        assert!(open("/dev/picabia-no-such-port", 0).is_err());
    }

    /// Lists the real ports (never opens one). Ignored: depends on the machine.
    #[test]
    #[ignore]
    fn lists_ports() {
        for p in list_ports() {
            eprintln!("{} | {}", p.path, p.name);
            if cfg!(target_os = "macos") {
                assert!(!p.path.starts_with("/dev/tty."));
            }
        }
    }
}
