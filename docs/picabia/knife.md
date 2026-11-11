# Drag knife

**Status:** 2026-11-10, PIC35. Built and tested against the simulated GRBL machine only; never run
on a real machine or used to cut anything. Every depth and speed is a guess until cut.

The laser machine (the owner's X-Carve) can hold a swivel drag knife on its Z axis instead of the
laser. The machine profile's **Head** says which one is in the clamp, and jobs are written and
checked for it.

## Using it

1. Window ▸ Laser ▸ Machine profile: **Head ▸ Drag knife**, then the knife's settings (below).
2. Connect. Jog to the sheet's bottom-left corner. Lower the head with the **Z** buttons (steps of
   at most 10 mm a click; use 1 or 0.1 near the sheet) until the blade just touches the sheet.
3. **Set origin here**. With the knife this sends `G10 L20 P1 X0 Y0 Z0`: X, Y and Z 0 are the
   sheet's bottom-left corner on its surface.
4. Frame (the head goes up to the travel height first, then traces the box), then Start. The
   checklist is the knife's: the knife is in the clamp and the laser unplugged, Z was zeroed on
   the sheet, the material is held down, hands stay clear, someone stays, an emergency stop is in
   reach.

Knife jobs and frames are refused until Set origin has zeroed Z with the knife as the head **on
this connection**. GRBL keeps a work offset across connections and power cycles, so an older
offset (the laser's, say) could put Z 0 anywhere, and the job's first `G0 Z5` would not be 5 mm
above the sheet. Reconnecting forgets it.

## Settings (`laser.profile` › `knife`)

| Setting | Key | Default (a guess) | Range |
|---|---|---|---|
| Travel height | `safeZMm` | 5 mm | 0.5–50 |
| Cut depth (Z) | `cutZMm` | −0.5 mm | −10–0 |
| Score depth (Z) | `scoreZMm` | −0.2 mm | −10–0 |
| Cut score lines | `score` | on | |
| Cut speed | `feedMmMin` | 1500 mm/min | 1–20000 |
| Plunge speed | `plungeMmMin` | 300 mm/min | 1–5000 |
| Passes (cut) | `passes` | 1 | 1–20 |
| Blade offset | `offsetMm` | 0.25 mm | 0–2 (0: none) |
| Swivel over | `swivelDeg` | 15° | 0–180 |
| Overcut | `overcutMm` | 1 mm | 0–10 |

The cut depth is how far below the surface the holder is pushed. A spring-loaded holder's blade
exposure, not Z, sets how deep the blade goes, so the depth only has to be enough to press it.

The kerf is 0 for knife jobs: a blade takes no width out, so the material's beam kerf would grow
every cut-out. A kerf given to the command still wins.

## What the G-code does (`vectorcraft_stencil::knife_gcode`)

- `G21 G90 M5`, then `G0 Z<travel>` before any move across the sheet.
- Score lines first, at the score depth, once. Then the cut in the sheet's planned order (inner
  contours first, the outline last). Engraving (the sheet's label) is left out, with a warning.
- Each path and each pass: `G0` to the start at the travel height, `G1 Z<depth> F<plunge>`, the
  cut, `G0 Z<travel>`.
- **Blade offset.** The tip trails the holder's axis by the offset. So the axis runs that far ahead
  of every point, and at a corner turning more than the swivel angle it goes round the corner on a
  `G2`/`G3` arc of that radius, which turns the blade where it stands. Gentler corners get a
  straight chord. An arc that would round onto its own start (GRBL reads that as a whole circle)
  is written as a line.
- **Overcut.** A closed contour runs on past its start along itself, so the first, still-turning
  part of the cut is cut again.
- The end: up, `G0` to the origin, `M5`. The only words are `G0 G1 G2 G3 G21 G90 M5 F X Y Z I J`.
  Nothing turns the spindle output on.

## Guards

- **Allow-list.** With the knife as the head, every job line (Picabia's own or a file) is checked
  against the knife's list. It is the laser's list (`check_line`), except that Z is allowed and
  `M3`, `M4` and an `S` above 0 are refused. The laser may still be wired to the spindle output,
  and a knife job that turned it on would burn whatever is under the head. A laser file opened by
  mistake is refused at its first `M4`. The laser's own list still refuses Z.
- **The head comes from the profile.** `head` isn't a job param, so a job can't pick the other
  tool.
- **`$32 = 1` is still required.** With laser mode on, GRBL never powers the spindle output during
  moves, even if a line turned it on.
- The power ladder is refused for the knife (it measures beam power). Framing with the beam is
  refused for the knife.

## Gaps

- Never connected to a real machine or cut. All defaults are guesses (`checked: false`).
- The knife's settings are one set per machine, not per material.
- The job's Z range isn't checked against the machine's Z travel (`$132`): soft limits (`$20`) are
  the only guard against a travel height above the top of Z.
- Open score lines get no overcut, and every path starts with the blade pointing wherever it was
  (no lead-in to turn it first), so the first offset's length of each path is cut by a turning
  blade.
- No tangential (driven) knife.
- The SVG export (for LightBurn) is the laser's. The knife is G-code only.
- Frames lift Z at the travel speed; GRBL caps it at the Z axis's maximum rate (`$112`).
- The new window strings are English only.
