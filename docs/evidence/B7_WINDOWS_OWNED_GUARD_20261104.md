# B7 Windows owned input guard — 2026-11-04

Stage 1 blocker B7, closed. `tools/remote-edit/hil-guard.ps1 -OwnerReady
-Unattended` ran on this laptop at main `982e6a9`, with the owner's desktop
reservation (hands off for the run), using freshly built
`build.ps1 build-remote-helpers` artifacts:

- `vw-input-helper.exe` SHA-256 `ce176f3acb4c94a5f298c3bbe50b8d51292fe147ab34275461453cefdc8c1b9b`
- `vw-remote-guard-hil.exe` SHA-256 `cd7a47561fe20015f56a9755e762f3173f8b63fc335430593cebe38a598eb6c8`

## Result

The receipt `.local/remote-guard-9f96ff8968f24ff5a1ce9ca37dee112f/receipt.json`
(SHA-256 `58117e56…165a`) reports **scoped_harness_passed=true** and
**cleanup_confirmed=true**, with 0 screenshots. Its `cases.jsonl`
(`6b4d7589…7d7f`) holds 103 case records:

| Case | Count |
| --- | --- |
| delivered_pen_click_wheel (paired click, wheel, sink unchanged) | 1 |
| min_wheel_refused_and_sealed (returned Invalid, owner retired) | 1 |
| target_mutation_latched_fresh_grant (move/resize/minimize/switch, fresh grant required) | 100 |
| explicit_cancel_retires_actual_helper | 1 |

The production helper exited 0 after 36.3 s. Every focus hand-off went only
to the harness's own two windows.

## Why 2026-10-20 failed

The earlier attempt refused at initial readiness because an unrelated app held
the foreground: the guard deliberately refuses to take focus from a foreign
window. With the owner's desktop reservation it ran clean, so the refusal was
not a code defect and no code changed.

## Limits

This was software-generated, unattended readiness. Physical mouse and S Pen
provenance, editor fidelity, latency and the integrated phone link are not
covered.
