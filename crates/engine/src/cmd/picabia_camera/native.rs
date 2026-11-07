//! PICABIA: the camera link's worker thread and the transport it owns (desktop builds only).
//!
//! Every turn the worker takes the commands' messages, then pumps the controller once (a request
//! and its answer, then the datagrams that came, waiting at most 20 ms), then publishes the state
//! and, when a new frame arrived, the frame (its JPEG, and decoded for the Camera window). Closing
//! never waits on the camera: the worker sends a last `stopstream` on its own and ends.

use std::net::IpAddr;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use vectorcraft_lumix::frame::MAX_DATAGRAM;
use vectorcraft_lumix::net::NetTransport;
use vectorcraft_lumix::sim::{SimCamera, SimOptions};
use vectorcraft_lumix::{CaptureOutcome, Controller, Options, Phase, Snapshot, Transport, pump};

use super::CameraFrame;

/// How long a message waits for the worker's answer.
const REPLY_TIMEOUT: Duration = Duration::from_millis(1500);
/// The widest and tallest frame decoded.
const MAX_FRAME_SIDE: u32 = 4096;
/// The simulated camera's picture.
const SIM_W: u32 = 640;
const SIM_H: u32 = 480;

/// What to connect to.
pub enum Target {
    Camera(IpAddr),
    Sim(SimOptions),
}

enum Msg {
    LiveView(bool),
    Capture(Sender<Result<u64, String>>),
    Close,
}

#[derive(Default)]
struct Shared {
    snap: Snapshot,
    /// The newest frame's number and JPEG.
    jpeg: Option<(u64, Arc<Vec<u8>>)>,
    decoded: Option<CameraFrame>,
    /// The last capture's number and what became of it.
    capture: Option<(u64, CaptureOutcome)>,
    /// The worker ended on its own (why).
    ended: Option<String>,
}

/// The worker (see the module docs).
pub struct Worker {
    tx: Sender<Msg>,
    shared: Arc<Mutex<Shared>>,
    ip: String,
    sim: Option<SimCamera>,
}

impl std::fmt::Debug for Worker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Worker").field("ip", &self.ip).field("sim", &self.sim.is_some()).finish()
    }
}

fn lock(m: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Worker {
    /// Open the transport and start the worker; `ip` is what the status shows.
    pub fn start(target: Target, ip: &str, device_id: &str) -> Result<Worker, String> {
        let (transport, sim): (Box<dyn Transport + Send>, Option<SimCamera>) = match target {
            Target::Camera(addr) => (Box::new(NetTransport::open(addr, vectorcraft_lumix::DEFAULT_UDP_PORT)?), None),
            Target::Sim(opts) => {
                let cam = SimCamera::new(opts);
                (Box::new(cam.clone()), Some(cam))
            }
        };
        let opts = Options { udp_port: transport.udp_port(), device_id: device_id.to_string(), ..Options::default() };
        let (tx, rx) = channel();
        let shared = Arc::new(Mutex::new(Shared::default()));
        let sh = shared.clone();
        std::thread::Builder::new()
            .name("picabia-camera".into())
            .spawn(move || run(Controller::new(opts), transport, &rx, &sh))
            .map_err(|e| format!("can't start the camera link: {e}"))?;
        Ok(Worker { tx, shared, ip: ip.to_string(), sim })
    }

    /// End the worker (it sends a last `stopstream` on its own).
    pub fn close(self) {
        let _ = self.tx.send(Msg::Close); // a worker that already ended needs nothing
    }

    pub fn live_view(&self, on: bool) -> Result<(), String> {
        self.tx.send(Msg::LiveView(on)).map_err(|_| self.gone())
    }

    /// Fire the shutter and wait (at most `timeout_ms`) for the camera to say it fired. The
    /// live-view frame from just before, when live view is running.
    pub fn capture(&self, timeout_ms: u64) -> Result<Option<Arc<Vec<u8>>>, String> {
        let frame = {
            let sh = lock(&self.shared);
            let live = sh.snap.phase == Phase::Streaming && !sh.snap.no_frames;
            sh.jpeg.as_ref().filter(|_| live).map(|(_, j)| j.clone())
        };
        let (reply, answer) = channel();
        self.tx.send(Msg::Capture(reply)).map_err(|_| self.gone())?;
        let id = answer.recv_timeout(REPLY_TIMEOUT).map_err(|_| self.gone())??;
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        while Instant::now() < deadline {
            match lock(&self.shared).capture.as_ref() {
                Some((n, CaptureOutcome::Done)) if *n == id => return Ok(frame),
                Some((n, CaptureOutcome::Failed(why))) if *n == id => return Err(format!("{why}; nothing was saved")),
                _ => {}
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Err(format!("the camera didn't confirm the capture within {} s; nothing was saved", timeout_ms / 1000))
    }

    /// The newest frame's number and JPEG.
    pub fn jpeg(&self) -> Option<(u64, Arc<Vec<u8>>)> {
        lock(&self.shared).jpeg.clone()
    }

    pub fn decoded(&self) -> Option<CameraFrame> {
        lock(&self.shared).decoded.clone()
    }

    fn gone(&self) -> String {
        lock(&self.shared).ended.clone().unwrap_or_else(|| "the camera link has ended: connect again".into())
    }

    pub fn status_json(&self) -> Value {
        let sh = lock(&self.shared);
        let s = &sh.snap;
        let camera: serde_json::Map<String, Value> = s.state.iter().filter(|(k, _)| k != "result").map(|(k, v)| (k.clone(), json!(v))).collect();
        json!({
            "connected": sh.ended.is_none(),
            "phase": if sh.ended.is_some() { "closed" } else { s.phase.as_str() },
            "ip": self.ip,
            "sim": self.sim.is_some(),
            "liveView": s.live_view,
            "fps": s.fps,
            "frames": s.frame_seq,
            "dropped": s.dropped,
            "lastDrop": s.last_drop,
            "noFrames": s.no_frames,
            "hint": hint(s, &self.ip, sh.ended.as_deref()),
            "lastError": sh.ended.clone().or_else(|| s.last_error.clone()),
            "failures": s.failures,
            "retryInSec": s.retry_in_ms.map(|ms| (ms as f64 / 100.0).ceil() / 10.0),
            "handshaken": s.handshaken,
            "camera": camera,
            "log": s.log,
        })
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Close); // the worker may have ended already: nothing to stop
    }
}

/// What to try, in plain words.
fn hint(s: &Snapshot, ip: &str, ended: Option<&str>) -> Option<String> {
    if ended.is_some() {
        return Some("Connect again.".into());
    }
    if s.phase == Phase::Lost {
        return Some(format!(
            "No answer from {ip}. Is the camera on with its Wi-Fi remote control waiting for a smartphone, and is this computer on the camera's network?"
        ));
    }
    if s.no_frames {
        return Some("Live view isn't arriving. On macOS, allow Picabia under System Settings ▸ Privacy & Security ▸ Local Network, and check that no firewall blocks incoming UDP.".into());
    }
    None
}

/// The worker loop.
fn run(mut ctl: Controller, mut t: Box<dyn Transport + Send>, rx: &Receiver<Msg>, shared: &Mutex<Shared>) {
    let t0 = Instant::now();
    let now = move || u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX);
    let mut buf = vec![0u8; MAX_DATAGRAM];
    let mut last_seq = 0;
    let mut capture_id = None;
    ctl.connect(now());
    // A panic here would only end this thread; catching it lets the status say so.
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        loop {
            if !take_messages(&mut ctl, rx, &mut capture_id) {
                ctl.disconnect();
                if let Some(req) = ctl.poll(now()) {
                    let _ = t.get(&req.path); // best effort: the camera may be gone already
                }
                return;
            }
            pump(&mut ctl, t.as_mut(), &mut buf, 20, &now);
            publish(&ctl, shared, &mut last_seq, capture_id, now());
        }
    }));
    let mut sh = lock(shared);
    sh.snap = ctl.snapshot(now());
    sh.ended = Some(if r.is_err() { "the camera link failed (internal error): connect again".into() } else { "disconnected".into() });
}

/// Handle the waiting messages; `false`: close.
fn take_messages(ctl: &mut Controller, rx: &Receiver<Msg>, capture_id: &mut Option<u64>) -> bool {
    loop {
        match rx.try_recv() {
            Ok(Msg::LiveView(true)) => ctl.start_live_view(),
            Ok(Msg::LiveView(false)) => ctl.stop_live_view(),
            Ok(Msg::Capture(reply)) => {
                let r = ctl.capture();
                if let Ok(id) = r {
                    *capture_id = Some(id);
                }
                let _ = reply.send(r); // the command may have given up waiting
            }
            Ok(Msg::Close) | Err(TryRecvError::Disconnected) => return false,
            Err(TryRecvError::Empty) => return true,
        }
    }
}

fn publish(ctl: &Controller, shared: &Mutex<Shared>, last_seq: &mut u64, capture_id: Option<u64>, now: u64) {
    let fresh = (ctl.frame_seq() != *last_seq).then(|| ctl.frame().map(|f| Arc::new(f.jpeg.clone()))).flatten();
    // Decoded outside the lock: the window reads the last one meanwhile.
    let decoded = fresh.as_ref().and_then(|j| decode(ctl.frame_seq(), j));
    let mut sh = lock(shared);
    sh.snap = ctl.snapshot(now);
    if let Some(j) = fresh {
        *last_seq = ctl.frame_seq();
        sh.jpeg = Some((*last_seq, j));
        if decoded.is_some() {
            sh.decoded = decoded;
        }
    }
    if let Some(id) = capture_id {
        sh.capture = ctl.capture_outcome(id).map(|o| (id, o));
    }
}

/// A JPEG as RGBA8, its size capped.
fn decode(seq: u64, jpeg: &[u8]) -> Option<CameraFrame> {
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(jpeg), image::ImageFormat::Jpeg);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_FRAME_SIDE);
    limits.max_image_height = Some(MAX_FRAME_SIDE);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let img = reader.decode().ok()?.to_rgba8();
    let (width, height) = img.dimensions();
    Some(CameraFrame { seq, width, height, rgba: Arc::new(img.into_raw()) })
}

/// The simulator's options from `camera.connect`'s `sim`, on a real clock, drawing a test card.
pub fn sim_options(p: Option<&Value>) -> Result<SimOptions, String> {
    let t0 = Instant::now();
    let mut o = SimOptions {
        fps: 15,
        clock: Arc::new(move || u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX)),
        real_time: true,
        frame: Arc::new(test_card),
        ..SimOptions::default()
    };
    let Some(p) = p else { return Ok(o) };
    let flag = |k: &str| -> Result<Option<bool>, String> {
        match p.get(k).filter(|v| !v.is_null()) {
            None => Ok(None),
            Some(v) => v.as_bool().map(Some).ok_or_else(|| format!("`sim.{k}` must be true or false")),
        }
    };
    let num = |k: &str, lo: f64, hi: f64| -> Result<Option<f64>, String> {
        match p.get(k).filter(|v| !v.is_null()) {
            None => Ok(None),
            Some(v) => v.as_f64().filter(|x| x.is_finite() && (lo..=hi).contains(x)).map(Some).ok_or_else(|| format!("`sim.{k}` must be {lo}-{hi}")),
        }
    };
    o.require_handshake = flag("requireHandshake")?.unwrap_or(false);
    o.offline = flag("offline")?.unwrap_or(false);
    o.refuse_capture = flag("refuseCapture")?.unwrap_or(false);
    if let Some(f) = num("fps", 1.0, 60.0)? {
        o.fps = f as u32;
    }
    if let Some(ms) = num("streamTimeoutMs", 500.0, 600_000.0)? {
        o.stream_timeout_ms = ms as u64;
    }
    Ok(o)
}

/// The simulated camera's picture for frame `n`: a soft gradient with a bar that sweeps across,
/// so live view visibly moves. Encoded as a real JPEG.
pub fn test_card(n: u64) -> Vec<u8> {
    let bar = u32::try_from(n.wrapping_mul(8) % u64::from(SIM_W)).unwrap_or(0);
    let mut rgb = Vec::with_capacity((SIM_W * SIM_H * 3) as usize);
    for y in 0..SIM_H {
        for x in 0..SIM_W {
            let on_bar = x >= bar && x < bar + 24;
            let px = if on_bar { [240, 90, 60] } else { [(x * 200 / SIM_W) as u8 + 30, (y * 200 / SIM_H) as u8 + 30, 150] };
            rgb.extend(px);
        }
    }
    let mut out = Vec::new();
    let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 70);
    if image::ImageEncoder::write_image(enc, &rgb, SIM_W, SIM_H, image::ExtendedColorType::Rgb8).is_err() {
        return vectorcraft_lumix::sim::stand_in_jpeg(n);
    }
    out
}
