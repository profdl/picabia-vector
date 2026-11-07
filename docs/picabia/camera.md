# Camera link (Panasonic Lumix GX85)

**Status:** 2026-11-06. Built and tested against the simulated camera (PIC34.1 to PIC34.6, PIC34.9),
**not yet tried on a real camera**: the protocol below is still a set of hypotheses from community
notes on Panasonic's Wi-Fi remote control. The probe (PIC34.0) is written; running it on the owner's
DMC-GX85 is the next step, and this page is rewritten from its log. Full-resolution download
(PIC34.7) and lock focus and exposure (PIC34.8) wait for that log.

Use a Lumix GX85, mounted once, as a fixed animation-stand camera: see its live view in a Camera
window, fire the shutter from Picabia, and collect numbered frames in a folder you choose,
optionally placing each one on the canvas as a linked image.

## Decisions

Agreed with the owner on 2026-11-06:

| Question | Decision |
|---|---|
| Where the camera code runs | In the desktop app, native only. Picabia talks HTTP and UDP to the camera itself: no bridge process, no Node (the repo is Rust only). The web build compiles the crate without its socket code, and its Camera window says the camera needs the desktop app. |
| How it attaches to Picabia | Like the GRBL laser link: a standalone Picabia crate, `camera.*` engine commands in Picabia-only files, a Camera window, and a few one-line `// PICABIA:` seams listed in [`SEAMS.md`](SEAMS.md). |
| Where live view shows | In the Camera window only. Nothing is drawn on the canvas. |
| What Capture produces | A JPEG in the capture folder (the default). An option in the window also places each capture on the canvas as a **linked** image. |
| Capture folder | **The user chooses it** with the folder picker. There is no built-in default location: the first Capture with no folder set opens the picker, and a cancelled picker captures nothing. The choice is remembered in the preferences. |
| Diagnostic probe | An example binary in the crate (`cargo run -p vectorcraft-lumix --example probe`), so it touches no upstream file. |
| Networking | Both ways: the Mac on the camera's own Wi-Fi (camera at `192.168.54.1`, the default) or the camera on the studio network (IP typed in, or found with **Find**). |
| First slice | Shutter, plus saving the live-view frame (about 640×480) to the folder as a stand-in. Downloading the full-resolution JPEG comes in phase 2. |
| Extras | Numbered frame sequence; a capture shortcut and Stop; locking exposure and focus. |

## Trying it without a camera

Open Window ▸ Camera…, type `sim` as the camera's address and press Connect. The simulated camera
streams a moving test card at 15 fps and answers Capture like the real one.

From the control channel or MCP:
`camera.connect {"ip": "sim"}`, then `camera.settings {"folder": "/some/folder"}`, then
`camera.capture`. With `sim` you can also ask for misbehaviour: `{"requireHandshake": true}`,
`{"offline": true}`, `{"refuseCapture": true}`, `{"fps": 30}`, `{"streamTimeoutMs": 3000}`.

## At the studio: run the probe (PIC34.0)

1. On the GX85, start Wi-Fi ▸ Remote Shooting & View (the camera waits for a smartphone), and join
   the Mac to the camera's Wi-Fi network.
2. Run:
   ```
   cargo run -p vectorcraft-lumix --example probe -- --out ~/Desktop/gx85-probe --capture --measure-timeout
   ```
   Add `--handshake` if the commands are refused, and `--ip …` if the camera is on another network.
   With the camera on the studio network, `--discover` also searches with SSDP and saves each
   device description, which phase 2 needs.
3. If macOS asks whether Terminal may find devices on the local network, allow it. With no
   datagrams at all, check System Settings ▸ Privacy & Security ▸ Local Network.
4. Bring back `probe.log` and the `frame_*.jpg` files beside it.

## Protocol (hypotheses until PIC34.0)

- **Commands** are HTTP GETs on port 80 of the camera:
  - `/cam.cgi?mode=camcmd&value=recmode` switches to record mode.
  - `/cam.cgi?mode=camcmd&value=capture` fires the shutter.
  - `/cam.cgi?mode=startstream&value=<udpPort>` starts live view to that UDP port.
  - `/cam.cgi?mode=stopstream` stops it.
  - `/cam.cgi?mode=getstate` returns status and also serves as the keep-alive.

  Replies are small XML documents, `<camrply><result>ok</result>…</camrply>`.
- **Access handshake.** Some firmware refuses commands until it gets
  `/cam.cgi?mode=accctrl&type=req_acc&value=<uuid>&value2=<deviceName>`. The controller tries the
  commands without it first, and sends it once if one is refused with `err_unsuitable_app`,
  `err_reject` or `err_non_support_app`. The uuid is made once and kept in the preferences, so the
  camera sees the same device every time.
- **Live view** arrives as UDP datagrams sent to the Mac's address on the port we asked for. Each
  one is a binary header followed by a JPEG; the frame is taken from the first `FF D8` through the
  last `FF D9`. The probe hex-dumps the first headers.
- **Keep-alive.** The stream stops after a few seconds unless it's refreshed. The link sends
  `getstate` every 5 s and `startstream` every 10 s, and sends `startstream` again at once when no
  frame has come for 3 s. PIC34.0 measures the real timeout.
- **Full resolution (phase 2).** After a capture the JPEG is on the SD card. It can be fetched
  through the camera's DLNA/UPnP ContentDirectory service (port 60606 on many Lumix bodies).
- **Camera settings (phase 2).** Focus and exposure through `mode=setsetting` calls.
- **Discovery.** When the camera joins another network, it should answer an SSDP M-SEARCH as a
  media server, naming Panasonic in its `SERVER` header. Find lists everything that answers, the
  likely cameras first.

Sources: the public community notes this page relies on are cited by URL here once PIC34.0 has
checked them. This is Panasonic's protocol, not Adobe's, so the clean-room rule doesn't apply to
it. Still, only public notes and our own probe of the owner's camera are used.

## How it's built

### `crates/lumix` (`vectorcraft-lumix`, standalone, no dependencies)

- **`Controller`**, a sans-IO state machine: connect (`getstate`, the handshake if it's needed,
  `recmode`, `startstream`), keep-alive, reconnect with backoff (1 s doubling to 30 s) while the
  link is open, capture, live view on and off, disconnect (one best-effort `stopstream`). Time
  comes in as `now_ms`.
- **`frame`** gets the JPEG out of a datagram. Datagrams over 64 KiB, with no markers, or shorter
  than 64 bytes are dropped and counted, and nothing indexes without a check.
- **`reply`** reads `cam.cgi` XML leniently (leaf elements by name, at most 16 KiB and 64 fields).
- **`sim`**: the simulated camera, behind the same `Transport` as the real one.
- **`ssdp`**: the M-SEARCH request and reading its answers.
- **`net`** (native only): HTTP/1.1 GET over `TcpStream` (2 s to connect, 3 s to answer, at most
  512 KiB, chunked bodies decoded), live view on a `UdpSocket` that keeps only the camera's
  datagrams, SSDP discovery, and `fetch` for device descriptions.
- **`examples/probe.rs`**: PIC34.0's probe.

### Engine: `crates/engine/src/cmd/picabia_camera.rs` (Picabia-only files)

A worker thread owns the transport and the controller. It decodes each new frame once (at most
4096 px on a side) for the Camera window. The link's state is the `camera` field of
`PicabiaState`, which lives in a Picabia file.

| Command | Params | What it does |
|---|---|---|
| `camera.status` | | State, address, fps, frames, dropped datagrams, a plain hint (no answer, no frames: Local Network permission), the camera's `getstate` values, folder, prefix, next file name, the place option, and the last 50 log lines. |
| `camera.connect` | `{ip?, sim?}` | Connects and starts live view. The default address is the last one used, else `192.168.54.1`. `sim` is the simulated camera. |
| `camera.disconnect` | | Asks the camera to stop streaming and closes the link. |
| `camera.start` / `camera.stop` | | Live view on or off; the link stays connected, and captures work either way. |
| `camera.capture` | | Takes the newest frame, fires the shutter, and once the camera confirms (at most 5 s), saves the frame as `<prefix>_<NNNN>.jpg`. It never replaces a file. With live view off it fires but saves nothing (and says so); a refused shutter saves nothing. It never touches the document. |
| `camera.settings` | `{folder?, prefix?, place?}` | The folder must exist and be writable. The prefix is up to 40 letters, digits, spaces, `-` or `_`. Remembered in `Prefs::picabia_camera`. |
| `camera.frame` | | The newest frame as base64 JPEG (lets agents see live view). |
| `camera.find` | `{timeoutMs?}` | SSDP search. |

**Real cameras need the network switch.** The desktop app turns it on at startup. The CLI and MCP
can turn it on with `PICABIA_CAMERA_NETWORK=1`. Without it, only `sim` connects, so tests and the
command sweep never touch the network.

### UI: `crates/ui-egui/src/picabia_camera_window.rs` (Picabia-only file)

Window ▸ Camera… (`camera.panel`) opens a floating window with:

- the live view;
- a status line and a hint when something's wrong;
- the address with Connect / Disconnect and Find (the cameras found can be clicked);
- the folder with Choose…, the prefix and the next file name;
- **Place on canvas as linked image**;
- **Capture**, and Start / Stop Live View.

There are two UI commands:

- **`ui.cameraCapture` (Capture Frame to Folder).** Opens the folder picker first when no folder is
  set, captures, and with Place on, places the frame fitted inside the active artboard as a
  linked image (`file.place` with `link: true`). Bind it to a key in the shortcut preferences for a
  capture shortcut: it's unbound by default.
- **`ui.cameraFolder` (Choose Capture Folder…).**

Find waits for its 1.5 s search, so the window pauses that long.

## Phases

| Task | What | Status |
|---|---|---|
| **PIC34.0** | The probe example. | Written. Runs, and fails cleanly with no camera. **Next: run it on the GX85** (see above). |
| **PIC34.1** | Crate: controller, frame, reply, simulator. | Done. Unit tests and random-input tests (no panics, sizes capped). |
| **PIC34.2** | Native transport, keep-alive, reconnect, timeouts. | Done. Tested on loopback and with the simulator, including a minute of keep-alive with a 3 s camera timeout and a camera that goes offline and comes back. Not yet on a camera. |
| **PIC34.3** | Engine `camera.*`, preferences, numbered frames. | Done. Engine tests; the every-command sweep passes. |
| **PIC34.4** | Camera window, Window ▸ Camera…. | Done. Headless frame tests, and checked by screenshot in the app with the simulator. |
| **PIC34.5** | Capture: shutter plus frame, folder picker when none is set, linked image. | Done. Tested, and checked in the app with the simulator. |
| **PIC34.6** | Capture shortcut and Stop. | Done: `ui.cameraCapture` can be bound to a key, and `camera.stop` stops live view. |
| **PIC34.7** | Phase 2: full-resolution download over DLNA. | Waits for the probe's `--discover` output. |
| **PIC34.8** | Lock focus and exposure. | Waits for the probe. |
| **PIC34.9** | Find: SSDP discovery. | Done (`camera.find`, Find in the window). Not yet tried on a network with the camera on it. |

## Risks

- **macOS Local Network permission.** The first time the app receives UDP from the camera, macOS
  may ask for permission. If it's refused, no frames arrive, and after 3 s the window's hint says
  where to allow it.
- **No internet on the camera's Wi-Fi.** The Mac loses internet while it's on the GX85's own
  network (unless it's also on Ethernet). Having the camera join the studio network avoids this.
- **Unverified protocol.** If the probe finds live view too slow or unreliable, a later option is
  an HDMI capture card (the GX85 has micro-HDMI out) read as a UVC webcam, with Wi-Fi kept for the
  shutter and downloads.
- **Dependencies.** Phase 1 adds none: HTTP, UDP and SSDP use `std::net`, and the `cam.cgi` replies
  are read by hand. The DLNA replies in PIC34.7 are larger XML, which would be read with
  `roxmltree` (MIT OR Apache-2.0). It's already in `Cargo.lock` through `usvg`, so it adds no new
  third-party code.
