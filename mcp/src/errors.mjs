// What an agent is told when a tool call is refused. A refusal used to be a
// bare code ("package_changed"), which leaves an agent guessing. Each code now
// carries whether a retry can work and the next step to take.
//
// Every string here is fixed text. Nothing from the owner, a package or the
// file system is ever placed in an error result.

const again = 'retry';        // the same call can succeed if repeated
const refresh = 'refresh';    // repeat after reading fresh data first
const owner = 'ask_owner';    // only the owner can unblock it
const stop = 'do_not_retry';  // repeating the same call cannot succeed

const ADVICE = Object.freeze({
  invalid_request: [stop, 'An argument is missing, malformed or out of range. Copy ids, targets and hashes exactly as list_packages and recapture_package returned them, and pass exactly one of "text" or "image" to submit_result.'],
  busy: [again, 'Too many calls are running at once. Wait for your other calls to finish, then repeat this one.'],
  cancelled: [again, 'The call was cancelled before it finished. Nothing was changed; repeat it if you still need the result.'],
  package_not_published: [refresh, 'No published package has this id. Call list_packages and use an id from that list.'],
  target_required: [stop, 'This package is published for more than one model family. Pass "target" as shown by list_packages.'],
  package_changed: [refresh, 'The package no longer matches the identity you passed; the owner republished or removed it. Call list_packages again and use the current id, target and manifest_sha256.'],
  package_busy: [again, 'The package is in use by another operation. Repeat the call in a few seconds.'],
  package_invalid: [owner, 'The package files failed their integrity check, so nothing from it was returned. Ask the owner to publish the package again.'],
  package_path: [owner, 'The package does not contain the requested part in the expected place. Ask the owner to publish the package again.'],
  use_large_file_fallback: [stop, 'The package is too large to return inline. Use get_marker for the markers you need instead of get_package.'],
  response_too_large: [stop, 'The result is too large to return. Ask for less: one marker with get_marker, or a smaller list_packages limit.'],
  verifier_unavailable: [owner, 'The package integrity checker could not be started. Ask the owner to check that Visual Workbench is installed correctly and running.'],
  verification_timeout: [again, 'Checking the package took too long. Repeat the call once; if it fails again, ask the owner to publish the package again.'],
  verification_too_large: [owner, 'The package is too large to check. Ask the owner to publish a smaller package.'],
  capture_grant_required: [owner, 'The owner has not granted this window to this connection, or the grant ran out. Ask the owner to grant the window in the Visual Workbench MCP settings; a grant lasts at most ten minutes.'],
  capture_grant_expired: [owner, 'The grant ran out while the capture was in progress. Ask the owner to grant the window again, then repeat the call.'],
  capture_not_ready: [again, 'The capture did not complete cleanly, so no package was published. Repeat the call.'],
  bridge_connection_required: [stop, 'This tool is only available through the stdio bridge started by Visual Workbench, not over HTTP. Tell the owner the result instead, or reconnect through the bridge.'],
  result_not_persisted: [again, 'The result was not saved. Repeat the call; if it fails again, tell the owner.'],
  owner_unavailable: [owner, 'Visual Workbench on the PC is not answering. Ask the owner to check that the desktop app is open.'],
  owner_timeout: [again, 'Visual Workbench did not answer within 30 seconds. Repeat the call once; if it fails again, ask the owner to check the desktop app.'],
  owner_busy: [again, 'Visual Workbench is still finishing earlier requests. Repeat the call in a few seconds.'],
  owner_refused: [owner, 'Visual Workbench refused the request without a stated reason. Ask the owner to look at the MCP panel in the desktop app.'],
  closed: [owner, 'The connection to Visual Workbench has closed. Ask the owner to reopen the desktop app and reconnect.'],
  comparison_changed: [refresh, 'The comparison no longer belongs to the receipt you passed. Call recapture_package again and use the new receipt.'],
  recapture_source_unavailable: [owner, 'The window or screen the original package was captured from is not available. Ask the owner to open it again, then repeat the call.'],
  tunnel_runtime_unavailable: [owner, 'Recapturing a phone screen needs a phone component that is not installed in this build. Ask the owner to verify this change on the phone.'],
  phone_disconnected: [owner, 'The original package was captured from the phone and no phone is connected. Ask the owner to connect it, then repeat the call.'],
  phone_route_unbound: [owner, 'The original package came from the phone, and the owner has not yet chosen which connected phone to recapture from. Ask the owner to bind the phone in the MCP panel.'],
  phone_mismatch: [owner, 'The connected phone is not the one the original package came from. Ask the owner to connect that phone.'],
  feedback_loop: [owner, 'Recapture was refused because the source currently shows the phone mirror window. Ask the owner to move or close the mirror window.'],
  source_busy: [again, 'The phone connection is busy or waiting for the owner to confirm something. Repeat the call in a few seconds.'],
  invalid_json: [owner, 'A file inside the package is not valid. Ask the owner to publish the package again.'],
  request_too_large: [owner, 'A file inside the package is larger than this server reads. Ask the owner to publish the package again.'],
  request_too_complex: [owner, 'A file inside the package is nested too deeply to read safely. Ask the owner to publish the package again.'],
  numeric_range: [owner, 'A file inside the package holds a number outside the supported range. Ask the owner to publish the package again.'],
  operation_failed: [owner, 'The call failed for an unexpected reason and nothing was returned. Repeat it once; if it fails again, tell the owner which tool failed.'],
});

/** The fixed advice for a refusal code; unknown codes get the generic entry. */
export function advice(code) {
  const [action, next_step] = ADVICE[Object.hasOwn(ADVICE, code) ? code : 'operation_failed'];
  return { error: code, action, next_step };
}

/** The tool result an agent receives for a refusal. The first text item is the
 * bare code so existing callers can still match on it. */
export function errorResult(code) {
  const detail = advice(code);
  return { isError: true, content: [{ type: 'text', text: code }, { type: 'text', text: detail.next_step }], structuredContent: detail };
}

export const ADVISED_CODES = Object.freeze(Object.keys(ADVICE));
