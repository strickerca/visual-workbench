# Performance measurement contract

Date: 2026-10-01. QUALITY-004 is partial. This file defines methods; no PERF
target has been measured or passed by T0.03. Static codec enumeration, a battery
snapshot and synthetic screenshots are not latency, thermal or recovery baselines.
Targets remain exactly those in REQUIREMENTS.json until T0.12 records a dated decision.

## Shared record and statistics

Every run records requirement/scenario, UTC date, operator, source/binary hashes,
app version and debug/release/profile configuration, phone model/Android build/API/
security patch, PC model/Windows build/driver version components, display physical
resolution/DPI/scale/refresh/advanced-color flags, document dimensions/format/hash/
layers/tiles/mask, tool versions, carrier/protocol, negotiated USB mode or Wi-Fi
band/channel/radio/signal strength (no addresses, SSIDs or serials), charging state,
battery level/temperature, ambient temperature, power mode, other load, cold/warm
cache state, instrumentation overhead and failures/cancellations.

Use monotonic nanosecond clocks (Windows QPC/Android elapsedRealtimeNanos); never
subtract wall-clock timestamps. Cross-device timestamps need at least 100
ping/pong clock-offset exchanges before and after each run; retain the chosen
offset interval, drift and worst-case synchronization uncertainty. Correlate
sequence/frame/operation IDs. If uncertainty crosses a target boundary, result
is inconclusive. Prefer a common 240 fps camera for end-to-end visible latency
when timestamps cannot establish presentation. One camera frame is about 4.17 ms;
record interval censoring and exposure/readout uncertainty. Compositor submission
or network arrival alone does not establish visible presentation.

Keep each raw duration, units, start/end event definitions, sample validity and
failure reason in text/CSV/JSON without private content. Sort valid samples
ascending; nearest-rank p50 is x[ceil(0.50*N)-1] and p95 is
x[ceil(0.95*N)-1]. Report N, min/max, p50/p95, failure count/rate, measurement
resolution and uncertainty. Never average p95 values; show each environment/
carrier/run separately, and label any pooled population. Do not discard slow
samples. Invalid instrumentation is retained and explained separately. Timeouts
are failures, not successful samples silently excluded. A small-N p95 is weak
evidence; repeated trials below are minimums, not release certification.

## PERF-001 — Phone pen contact to wet ink

Target: <=25 ms p95. On the actual S23 Ultra, film pen contact and first changed
wet-ink pixel with a 240 fps camera, or correlate MotionEvent eventTime/history
with actual presentation fences/validated frame timestamps. Use 100 strokes per
run, three runs after five labeled warmups; include slow/fast strokes and
interior/edge locations, with front-buffer/prediction settings recorded.
Compute p50/p95 for contact-to-first-visible-pixel, preserving prediction versus
measured contact evidence. Record pen tool type, pressure/tilt/hover availability,
display refresh, input sampling and renderer path. Future T0.04/T0.10 traces
establish feasibility; final product measurement needs the implemented wet-ink path.

## PERF-002 — Phone stroke visible on PC

Targets: <=50 ms p95 USB and <=80 ms p95 5 GHz Wi-Fi. Correlate actual phone
contact/input sample with first visible corresponding host stroke using a common
camera or synchronized input-to-present markers. Record batching, send/receive/
apply/render/present stages, frame IDs and clock uncertainty. Three runs of
100 strokes per carrier, five labeled warmups per run; calculate carrier-specific
p50/p95. Record protocol, RTT/loss/jitter, USB mode/Wi-Fi conditions, desktop
refresh/DPI and other app load. T0.06 supplies transport baselines; later stroke
sync measurements must include rendering rather than only packet delivery.

## PERF-003 — Remote edit round trip

Target: phone pen -> host render -> phone visible result <=80 ms p95 USB;
ghost ink immediate. Use sequence-bound pen events, host capture/render frames,
encode/send/decode stages and actual phone presentation, with camera validation.
Three runs of 100 edits plus five labeled warmups; p50/p95 on the complete
round trip, and separate ghost-ink latency using PERF-001's visible-pixel method.
Record creative app/version, input injection mode, capture backend, codec/profile/
resolution/bitrate, refresh, dropped/stale frames, prediction and clock uncertainty.
T0.07/T0.08 characterize components; product acceptance follows tunnel integration.

## PERF-004 — Freeze to lossless frame

Target: <=300 ms for a 4K window over USB. Time freeze request dispatch to phone
presentation of the hash-verified lossless frame for the selected physical
3840x2160 window. Record capture/readback, encode, transfer, decode and display;
confirm pixel equivalence and ensure a lossy preview is not counted as completion.
Thirty trials across three runs, cold and warm results separate; p50/p95 and
maximum, with the unqualified target assessed for each valid trial. Record
content fixture/hash, actual crop, DPI, codec, USB mode and clock uncertainty.

## PERF-005 — 200 MP import

Targets: PC preview <=1.5 s; complete PC pyramid <=6 s; phone screen fill <=250 ms.
Use a declared 200 MP fixture (exact pixel dimensions and hash), with monotonic
markers for import acceptance, first useful preview presented, all pyramid tiles
available, and phone fill request to required viewport tiles presented. Thirty
trials across three runs for each stage, separate cold/warm filesystem/cache
conditions; p50/p95 and maximum. The unqualified limits apply per valid trial.
Record source format/bit depth/orientation, decoder, pyramid levels/tile size/
compression, viewport/zoom, carrier, cache state and disk performance. Record
peak memory alongside duration; incomplete preview/pyramid is a failed trial.

## PERF-006 — Memory with a 200 MP document

Targets: PC <=1.2 GB working set; phone <=700 MB PSS. Use decimal bytes
(1,200,000,000 and 700,000,000), recording raw byte totals as well as MiB.
Run three 10-minute sessions spanning import, pan/zoom, ink, edits, export and
idle with a 200 MP document. Sample the exact owned PC process set's working
sets at 1 Hz; include host helpers, avoid double-counting shared pages when
reporting total versus per-process sums. Record process ownership and sampling
gaps. Collect phone PSS via Android memory APIs/dumpsys meminfo for the selected
package at 1 Hz when overhead permits; if slower, record cadence and missed
peaks. Report p50/p95 and highest observed values per phase/run, baseline/deltas,
sampling limitations and counters for allocation/high-water instrumentation.
The caps apply to peak measured memory, not just p95; 2 s build-runner samples
are not evidence of the product's worst-case peak.

## PERF-007 — Carrier switch and recovery

Target: sync resumes <=2 s after the new carrier becomes available; zero duplicate
commits. T0.06 records 10 USB unplug/replug trials and 10 Wi-Fi-drop/recovery
trials, plus 10 switches in each direction when both carriers exist. Record
physical disruption, OS link availability, tethering re-enabled (when required),
transport readiness, first successful acknowledged sync, and stable resumption.
Start the target clock at new-carrier availability (USB tethering: switched back
on), and report disruption-to-availability separately. Use monotonic/correlated
markers, p50/p95/max, failures and every recovery over 2 s. Verify a fixed
operation sequence/commit ledger before/after for zero duplicates, missing
commits and ordering; input replay is separately owned by TUNNEL-008.
Fields awaiting T0.06: trial_id, carrier_before/after, disruption_time,
available_time, tethering_enabled_time, sync_resume_time, recovery_ms,
duplicate_commit_count, missing_commit_count, clock_uncertainty_ms, result.

## PERF-008 — Export speed and cancellation

Targets: selection crop <=1 s; full 200 MP PNG <=20 s and cancellable. Time
accepted export request to completed, closed/flushed, readable output; define
any durability requirement separately. Validate dimensions/content/PNG integrity.
Thirty selection-crop and 30 full-image trials across three runs, recording
p50/p95/max and every unqualified target miss. Ten cancellation trials per
export type at early/middle/late phases; record request-to-stop time, UI
responsiveness, worker termination and partial-file cleanup. No invented
cancellation latency threshold: report measured values. Record selected area,
alpha/bit depth, encoder settings, destination class (no private path), disk/
cloud-sync state and concurrent work.

## PERF-009 — Masked AI composite

Targets: zero changed pixels outside the dilated mask on EVERY composite;
local processing <=2 s for 12 MP, excluding model/provider time. Use 30 declared
12 MP fixtures over three runs, including empty/full masks, edges, holes,
feathering, alpha, rotations and hostile oversized/incorrect provider results.
Freeze the canonical mask/dilation space and compare exact output versus input
bytes outside it (not screenshot appearance); retain changed-pixel count and
comparison hashes for every case. Time local validation/resize/color conversion/
masking/compositing through committed local result; report provider/network time
separately. p50/p95/max for local duration, every <=2 s miss and any changed
pixel as failure. Record mask hash/radius/coordinate space, image dimensions/
format/color profile, provider response shape, local backend and source hashes.

## Thermal baseline — awaiting T0.04

On the target phone, record ambient temperature, charging/battery state, initial
temperature, thermal status, display brightness/refresh, renderer/probe version
and exact session activity. Log battery temperature at 1 Hz for 600 s while the
owner draws continuously/casually, with monotonic elapsed times. Require 601
samples including t=0, label missed samples and stop/interruptions; record
start/end/min/max/delta and nearest-rank p50/p95, thermal status changes and
throttling. Repeat three sessions after returning to an explicitly recorded
baseline. A shared-phone interruption is labeled, not erased. T0.03's IN2019
28.4 C snapshot is not this S23 baseline. Raw fields: elapsed_s, temperature_c,
battery_percent, charging, thermal_status, activity_marker, validity/reason.

## Current environment and gaps

PC/phone facts are bound in pc-diagnostics.json, win-diagnostics.json and
phone-diagnostics.txt; hardware-truth.md distinguishes the S23 target from the
connected IN2019. Calibration/gamut, codec execution/low-latency, pen behavior,
thermal sessions, recovery, release-mode numbers and all nine product PERF
measurements remain untested. T0.04 and T0.06 fill baseline fields; T0.12 owns
dated target confirmation. Further feature owners execute the relevant methods.
Private recordings/screenshots must be inspected and disposed under the owner
retention rule, retaining only text counts/hashes/source bindings and receipts.
