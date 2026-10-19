# Assisted-selection license census, 2026-10-19

**Disposition: incomplete; no production model admission.** This E-lane source
checkpoint starts from integration `6efc8b8cba9daa4e4ff598e6419c4db41deb6a8b`
and the retained e28987e MobileSAM delivery. It adds a bounded offline audit,
machine-readable component/notice records and copied license or attribution
text. It does not alter the candidate catalog, converted graphs, root license
register, Android dependencies, runtime pins or production admissions.

The private input root remains `/workspace/assisted-model-delivery-20261019`.
The source-bound host delivery manifest is SHA-256
`a97d339dd323f6045a6bf10c54c2ea9e687799023869cd655459c13866f1014f`.
The model manifest is SHA-256
`7ea1a2fe7c979305e65372d85cb3255e49434667cfaa1c636c8d2052518af9f4`.
The graphs are encoder 28,142,464 bytes / SHA-256
`92f0ac2134bb88bb5e555cdb1dd7080bc4620fbe31790a799abcb4c26156f4d3`
and decoder 18,853,624 bytes / SHA-256
`9c0f89743e7056c18f4ee771e299d7db848aa14b500c025b452851fb3e2dbb78`.
These hashes identify artifacts; they do not grant rights to redistribute them.

## What was actually inspected

`tools/assisted-selection/license-audit/component-census.json` enumerates the
71 exact installed host wheels, all 258 recorded native files, all 114 original
wheel “notice” candidates, copied source text, model source/checkpoint and graph
identities. Each native row retains its path, size and SHA-256. Thirty-eight
native paths have a possible vendored/renamed component marker; every native
build/component license review remains open. Wheel bytes, selected native and
notice member bytes, metadata hashes, private notice copies and checked-in text
copies were reverified, not inferred from distribution names.

The inherited wheel inventory's broad `/licenses/` rule counted seven Python
modules as notices: two empty `kagglesdk` files and five nonempty `kagglesdk`,
`packaging` or vendored `setuptools` code files. The census retains those seven
recorded members with `license_or_attribution_text=false`; none was copied as
license text. Of the other 107 wheel entries, the audit copied 107 nonempty
license/attribution texts. Eleven pinned upstream/sdist texts bring the total
to 118 copied files under `third_party/notices/assisted-selection/`.
`notice-sources.json` records each copied file's exact source, size and hash.
No wheel, source archive, checkpoint, model or golden tensor was added to Git.

The seven wheels without a bundled license text have these source leads, not
completed native/product admissions:

| Host wheel | Recovered text | Remaining exact gap |
| --- | --- | --- |
| `ai-edge-litert` 2.2.0 | Apache-2.0 text from LiteRT tag `v2.2.0` (`145c7523ff08d5e57ab5c582141775eea47da9c7`). | Wheel has 22 native files, including vendor plugins; source/build and aggregate notices are unproved. |
| `flatbuffers` 25.12.19 | Apache-2.0 text from tag `v25.12.19` (peeled commit `7e163021e59cca4f8e1e35a7c828b5c6b7915953`). | No sdist for this PyPI release or wheel-to-tag build attestation was found. No native file is recorded. |
| `litert-converter` 0.4.0 | Wheel metadata declares Apache 2.0. | No exact upstream source/build mapping; six native files and aggregate notices remain unknown. No unrelated LiteRT tag is asserted as its source. |
| `litert-lm-builder` 0.18.0 | Apache-2.0 text from LiteRT-LM tag `v0.18.0` (`b2f686e2ed4718fb84ec398a61dd59ca0f0aff27`). | Wheel-to-tag build mapping and any package-specific notice remain unproved; no native file is recorded. |
| `ordered-set` 4.1.0 | MIT text from exact PyPI sdist SHA-256 `694a8e44c87657c59292ede72891eb91d34131f6531463aab3009191c77364a8`. | No native file; wheel does not bundle the text, and build provenance still needs review. |
| `sentencepiece` 0.2.2 | Root Apache-2.0 plus Abseil Apache-2.0, darts-clone BSD-3-Clause, esaxx MIT and protobuf-lite BSD-3-Clause texts from exact PyPI sdist SHA-256 `3d2b5e824b5622038dc7b490897efe05ebbbb9e7350fc142f3ecc8789ef9bdf6`. | One compiled native file; actual wheel build and complete third-party aggregate need review. |
| `tokenizers` 0.23.3 | Apache-2.0 text from exact PyPI sdist SHA-256 `cded33237c77caeef62944d32aa9a7ef42bdce2b3497e18d137e072a8c4be438`. | One Rust native file; its actual Cargo/vendor build and notices are unproved. |

The `litert-torch` and other 64 wheels with bundled texts are also host
conversion-only inputs, not Android product dependencies. Those texts are
retained and hash-bound, but the per-native and transitive notice review is
open. In particular, `.libs/` renames, `libgomp` and provider plugin names
cannot establish their build configuration or license on their own. The
standalone Android LiteRT 2.3.0 AAR/JNI/APK census belongs to root; host
LiteRT 2.2.0 does not close it.

## Model-specific rights gap

The ten pinned MobileSAM source files and root Apache-2.0 license are bound to
commit `f706ad9c4eb7f219c00d9050e46328518ffb65d2`. The TinyViT source
header separately attributes Microsoft 2022 and adaptations from LeViT/Swin;
its inherited component obligations need a reviewed mapping. The 40,728,226-byte
checkpoint (SHA-256
`6dbb90523a35330fedd7f1d3dfc66f995213d81b29a5ca8108dbcdd4e37d6c2f`)
contains no license/NOTICE-named ZIP entry. The pinned repository code license
does not alone establish redistribution terms for that checkpoint or the
derived encoder/decoder weights. Both graphs have `MLIR Converted.` and only
`min_runtime_version` and `keep_stablehlo_constant` metadata entries; neither
contains license or attribution metadata. They remain pending-admission.

## Root handoff and checks

`tools/assisted-selection/license-audit/root-component-proposals.json` supplies
11 exact `third_party/LICENSES`-shaped **candidate** entries with source URLs,
text hashes and narrow scope notes. These are separate from the root register;
host-only source texts should not be added to a product distribution simply
because conversion used them. The MobileSAM entry covers pinned source code
only. Root must obtain checkpoint-specific terms and resolve TinyViT lineage
before a model redistribution entry or catalog admission is justified.

From this worktree, the cheap offline validation is:

```sh
source /workspace/.vw-tools/cloud-setup/start.sh
python3 tools/assisted-selection/license-audit/prepare_sdists.py \
  --private-root /workspace/assisted-model-delivery-20261019
python3 tools/assisted-selection/license-audit/audit.py \
  --private-root /workspace/assisted-model-delivery-20261019
python3 tools/assisted-selection/license-audit/audit.py --source-only
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s tools/assisted-selection/license-audit -p 'test_*.py' -v
```

The first command is an optional networked preparation of three exact public
PyPI sdists, reused and hash-checked when already present. The full audit is
offline, uses no model interpreter or weights loader, and refuses
changed/missing wheel, native, notice, source, checkpoint or model bytes.
`--source-only` checks the committed wheel/notice census and source text when
the private artifacts are unavailable; it is not a replacement for the full
audit. Both deliberately output `production_admitted=false`. Six focused tests
cover changed/unlisted wheel components, missing/tampered notice text, sdist
tampering, changed census identity and unknown-license admission refusal. This
run passed the full 71-wheel audit, source-only check and all six tests.
No heavy application build, Android execution, model inference, device test or
license-gate approval occurred in this lane.
