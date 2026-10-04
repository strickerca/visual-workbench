# Codex App Server handoff candidate

Source-only T2.10 addition. This is a private desktop owner flow, not an MCP tool,
not a configured external client, and not evidence that a Codex turn was sent.
No installed Codex executable, credentials, provider, device, build, tests or
package installation was used while writing this candidate. Earlier reviewed
MCP R2/R4, owner, startup R2, catalog, capture and AI files remain unchanged.

## Implemented flow

The explicit Workbench menu opens a fenced owner panel. The owner chooses an
installed `codex.exe`; no executable search, fallback, shell argument, API key,
`CODEX_HOME`, `NODE_OPTIONS` or provider override is accepted. The UI reports that
checking the runtime starts its private App Server and uses that runtime's normal
account/configuration. Workbench itself does not read, display or persist its
credentials. Thread previews remain literal owner UI data and never enter logs.

The native helper pins the selected executable and directory ancestry, hashes
it, then starts the packaged Node adapter in a kill-on-close Job. The adapter
runs only `app-server generate-json-schema --out <private-directory>`, checks the
same executable hash again, compiles the emitted ClientRequest schema in a
bounded worker, and starts `app-server` over inherited stdio. Initialize precedes
initialized. Every request is checked against that installed schema. Turn input
fields and parameters must also be explicitly declared: a permissive old serde
schema cannot silently ignore `detail=original` or the reviewed model.

The owner explicitly lists existing threads, chooses an idle target, and loads
that exact thread without a path/history/model/security override. The response's
model and working directory are displayed. It never creates a thread. Schema
support alone does not assert any remote model's preprocessing behavior.

A generic/OpenAI package comes from the existing single catalog owner. The
native package verifier checks the whole immutable source; the adapter copies
all exact manifest-bound files into a new private stage, fsyncs each file, asks
the supervisor to pin the stage and its parents against writes/deletion, and
verifies the copy again. Package retirement cannot change these copied inputs.
No project files, source images or catalog entries are rewritten. The preview
contains exact text, package/hash identity, image dimensions/hashes and selected
thread/model. It lists image inventory; it does not claim to render/inspect all
package images or to prove that an external model has read them.

An owner checkbox plus Send passes the exact preview ID/digest. The attempt is
consumed before any possible turn/start write; an ambiguous response/cancellation
never rearms it. The model displayed in that receipt is supplied explicitly on
the actual turn, with the exact text and `localImage` paths. Idle is rechecked.
No approvals, password prompts or server-originated tool requests are approved:
they receive a fixed refusal. No sandbox/security policy override is sent.

Only the matching accepted turn is observed. Bounded early terminal receipts
handle completion arriving before turn/start's reply; unrelated thread/turn
notifications cannot become an accepted completion. A known turn can be explicitly
interrupted. Acceptance, completion and an uncertain delivery are distinct.
Closing the panel merely disposes its UI work. Stop/quit joins the private owner;
ending this App Server may stop its in-flight tools and cannot undo remote work.

## Image preprocessing receipt and honest default

`image-profiles.json` is a **packaging-pinned build input**, not editable owner
state or an agent-supplied option. The default has no profiles because this task
has not measured an installed runtime image path. Therefore image Send is
visibly refused by default; schema checking and existing-thread selection remain
real. MCP pull, Copy and file drag remain available independent alternatives.
This does not close the installed-client/image acceptance clause.

`image-profile.schema.json` documents the bounded input. Central verification
must provide the exact selected executable SHA-256, generated schema SHA-256,
returned model, supported detail spelling, maximum dimension, patch size/area,
verification/expiry times (at most 31 days), and a text-only evidence SHA-256.
The central evidence must bind the actual image preprocessing path and establish
that every admitted size remains unscaled for that runtime/detail/model. A
source-branch guess, schema-only success or arbitrary owner acknowledgment is
not such evidence. Keep that receipt with the central source/binary/build record.
Do not add a default model/profile merely to turn on the button. Both preview
and Send recheck this exact receipt; unsupported fields, missing/expired
receipts and dimensions exceeding its edge or patch budget refuse.

Official upstream sources show why a single edge bound is inadequate: a 2048²
image exceeds a 2500-patch budget with 32-pixel patches. These upstream constants
are not an assertion about the owner's installed executable. Current supported
schema variants and all receipt values must be established centrally before a
real explicit owner Send, never by a startup probe that transmits a package.

## Bounds and lifetime

One native Codex owner per application. Failed cleanup quarantines the retained
resource owner and admission slot until application exit. Concurrent/cancelled
close callers join one actual settlement. Process termination/reap happens
before pipe closure or runtime pin release; the native Job handle is destroyed
before file pins on all exits. The Job contains at most 16 processes, including
supervisor, Node, App Server and its agent tool descendants. This may refuse
larger tool trees; it is not a claim of unrestricted agent/tool compatibility.

Runtime hash: streaming 64 KiB buffer, executable <=256 MiB. Generated schema:
<=2048 filesystem entries, four nested levels, <=64 MiB total, <=4 MiB per file;
ClientRequest JSON <=32768 structural nodes/depth32. No external references,
regex patterns or remote schema resolution. AJV compilation has a 10-second
worker deadline and 192/16 MiB old/young heap limits; request validation two
seconds. These are process/admission caps, not hard OS allocation guarantees.

IPC frames <=2 MiB owner input and <=4 MiB runtime output; fixed bounded native
queues and a 15-second blocked-write watchdog. Schema generation gets 30 seconds
plus five seconds for cleanup, initialize 80 seconds total, each owner operation
120 seconds, runtime replies 30 seconds, final reap five seconds. Kotlin waits
90/130 seconds so native deadlines win. A cleanup timeout exits the owned
supervisor/Job instead of releasing a live child slot. No process-name kills,
normal application reset or global daemon manipulation exists.

At most two 32-MiB stages per process and 16 retained owner work directories.
The package maximum is 69 files (64 markers +5), including the manifest, and
66 image inputs. Private generated schemas/stages are retained after exit for
explicit recovery; this candidate has no broad or automatic cleanup. It does
not resume a stopped owner: the panel states that an application restart is
required. Unknown installed schema shapes fail closed rather than guessing.

## Literal integration and packaging

`integration/BASES.json` binds seven before/after pairs. Apply the literal hunks,
not full files, to ROOT **after** reviewed MCP startup R2. Main snapshots are
composition evidence: preserve all newer paid-AI close guards, instruction/focus,
selection/eraser, capture/picker, startup smoke and native-cache-anchor hooks.
Acquire the Codex modal fence synchronously before publishing panel state, and
retain it through the native executable chooser. Quit must join Codex before
releasing package/runtime/native pins. It opens no duplicate ProjectStore/catalog.

New source: six production JavaScript modules, one native binary, two Kotlin
production files, the packaging profile/schema and tests. No new dependency:
reuse approved Node24.21.0, AJV8.17.1 (MIT), sha2=0.11.0, serde/serde_json and
windows-sys=0.61.2 from reviewed inventories/lock/license evidence. This adapter
uses the emitted App Server JSON protocol directly; MCP protocol-version claims
remain in the separately tested MCP SDK2.3.0 candidate.

The literal Cargo addition builds `vw-codex-host` with the already approved
native manifest. Stage this binary and the seven required `mcp/codex` runtime
files beside the existing MCP host/verifier/Node/modules. Both generator and JVM
inventory require every exact new component, including the profile input. Reuse
the existing two-stage server-resources → app-image → final bridge recipe;
regenerate hashes after signing, before embedding them. Do not bypass the
existing native/core/capture inventory or build receipts. The existing startup
fixture receives only the matching required-file addition.

Parent alone runs formatting/compilers/licenses, then `node --test
mcp/codex/test_contract.mjs` in the pinned packaged dependency environment and
the desktop JUnit suite. There are 19 new Node and nine new JVM cases, all unrun
here. They cover installed-schema recognition, unknown-detail/model refusal,
exact one-shot receipt, malformed pending response, unreapable child deadline,
concurrent ownership/quarantine, early completion correlation, image expiry and
required runtime inventory. Parent must additionally exercise real Windows Job
nesting, executable pins, packaging, cancel/reap and the selected installed CLI
without sending. A real owner-approved Send and measured image proof are separate
acceptance; no code or tests here perform those actions.

## Primary source provenance

Local read-only inventory found the selected installation's `codex.exe` under
its versioned Codex bin directory. No local schema artifact was present; the
adapter generates it only when the owner explicitly requests runtime checking.
Official sources used for protocol shape and image-pipeline context:

- https://learn.chatgpt.com/docs/app-server (official OpenAI documentation reached
  from https://developers.openai.com/codex/app-server/): schema generation,
  stdio initialize/initialized, existing-thread resume and turn/start.
- https://github.com/openai/codex/blob/main/codex-rs/app-server-protocol/schema/json/ClientRequest.json
- https://github.com/openai/codex/blob/main/codex-rs/app-server-protocol/schema/json/v2/TurnStartParams.json
- https://github.com/openai/codex/blob/main/codex-rs/app-server-protocol/schema/json/v2/ThreadListResponse.json
- https://github.com/openai/codex/blob/main/codex-rs/app-server-protocol/schema/json/v2/ThreadResumeResponse.json
- https://github.com/openai/codex/blob/main/codex-rs/app-server-protocol/schema/json/v2/TurnStartResponse.json
- https://github.com/openai/codex/blob/main/codex-rs/protocol/src/user_input.rs
- https://github.com/openai/codex/blob/main/codex-rs/utils/image/src/lib.rs

Those moving upstream pages informed the implementation; only the installed
schema/hash and current central preprocessing receipt authorize its Send shape.

## R2 receipt ordering repair

The JVM keeps one structured receipt bound to the exact preview attempt, thread
and turn. Reply delivery and terminal notifications update the same atomic state;
a queued acceptance or interrupt acknowledgment cannot replace an observed
terminal receipt. The gated JVM regression completes a real Deferred reply,
holds caller dispatch, publishes completion, and then resumes the production
acceptance helper. Other cases refuse foreign attempt/thread/turn data and
retain terminal start replies. The startup fixture derives its count from its
explicit twelve-file required inventory. These source tests remain unrun.

R1's exact thirty files and manifest are preserved outside the repository at
`.handoffs/t2-codex-r1-preserved-20261003`; R2 changes no provider, executable,
external session or real Send state.
