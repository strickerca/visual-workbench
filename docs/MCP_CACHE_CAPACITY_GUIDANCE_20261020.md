# Explain bounded MCP cache capacity

The new Windows image (reported hash prefix `82ae4aa`, published source
`a89df0041dd17a2a85bbd679217ffec6b6681007`) refused `first_start` in
`bundle_extract` after 6 ms. Local inspection found exactly eight ordinary,
non-reparse cache children and no child for the requested manifest SHA256
`bbd54f8afd89296922856700503388ebe1a66741e5ba6e231e308dee772ede80`.
The manifest reports 1,514 files / 119,499,486 bytes. These are parent-reported
measurements; cloud has not inspected the raw receipt.

This proves the original intentional capacity check explains this refusal. The
hash counters measure later verification, not extraction amount. Persistent app
cache entries survive the smoke harness's private log/process cleanup. The older
image's second-start timeout remains a separate unresolved observation.

The app previously suggested rebuilding for every bundle refusal, which cannot
free a cache slot. This change preserves the original count and strict smoke
failure while carrying an internal `cache_capacity` detail only from the actual
capacity check. The existing app issue now explains the limit and asks the build
owner to review cache ownership and use before anything is removed. An existing
verified manifest key still reuses its bytes at capacity. No eviction, deletion,
limit increase, alternate runtime, new UI or verification bypass is implemented.

Eight new synthetic-resource cases verify the capacity boundary, unchanged prior
bytes, no payload open/new key, active leases, existing-key reuse, incomplete-key
marker failure, forged-detail rejection and existing app/smoke behavior. Fresh
focused compilation includes all 25 non-Compose MCP production sources so default
constructor changes do not rely on stale callers. JUnit runs 33 cases: 30 execute,
three existing actual-Windows methods stop at their assumptions, and none fail.
The collector's 41 tests pass. One delicate-API warning matches unchanged base
source. Dependencies are cached; this is not a fresh package or Windows run.
Independent Astra source review is clear. Exact source, test and log bindings are
in the [receipt](evidence/MCP_CACHE_CAPACITY_GUIDANCE_20261020.json).

Local owns cache disposition and disk headroom restoration. Close relevant
Workbench/agent owners and review exact cache ownership/use before removing any
entry. This patch frees no slot. Restore task-owned disk headroom without lowering
reserves, then rebuild and run the unchanged two-cycle smoke after a reviewed slot
is available. Neither the older timeout nor Windows acceptance is passed by the
guidance change. Cloud publishes a branch only; main promotion remains local.
