# MobileSAM host artifact delivery, 2026-10-19

The private transfer root is `/workspace/assisted-model-delivery-20261019`. The
canonical handoff is `artifacts/delivery-manifest-v3.json` (SHA-256
`a97d339dd323f6045a6bf10c54c2ea9e687799023869cd655459c13866f1014f`).
Its absolute paths and byte hashes bind every graph, golden, source input, wheel,
notice copy, tool source, and execution log. Copy that directory privately before
retiring this workspace. No model, checkpoint, wheel, golden tensor, or log is in
Git. The catalog still has no production admission.

| Private file | Bytes | SHA-256 |
| --- | ---: | --- |
| `artifacts/mobile-sam-fixed66/encoder.tflite` | 28,142,464 | `92f0ac2134bb88bb5e555cdb1dd7080bc4620fbe31790a799abcb4c26156f4d3` |
| `artifacts/mobile-sam-fixed66/prompt_decoder.tflite` | 18,853,624 | `9c0f89743e7056c18f4ee771e299d7db848aa14b500c025b452851fb3e2dbb78` |
| `artifacts/mobile-sam-fixed66/model-manifest.json` | 4,659 | `7ea1a2fe7c979305e65372d85cb3255e49434667cfaa1c636c8d2052518af9f4` |
| `inputs/mobile_sam.pt` | 40,728,226 | `6dbb90523a35330fedd7f1d3dfc66f995213d81b29a5ca8108dbcdd4e37d6c2f` |

The ten MobileSAM source and license inputs match commit
`f706ad9c4eb7f219c00d9050e46328518ffb65d2`. The converter installed 71
exact CPU-only wheels in a private Python 3.11.17 environment. The checked-in
`host-wheel-lock.json` records their bytes and hashes, 258 native files, and 114
bundled notice files copied to `artifacts/host-notices-v3`. Seven wheels lack a
bundled notice (`ai-edge-litert`, `flatbuffers`, `litert-converter`,
`litert-lm-builder`, `ordered-set`, `sentencepiece`, `tokenizers`); their source
and native license obligations remain for root review. The wheelhouse and copied
notices are private and have not been approved for redistribution.

`reference-parity-golden.json` records 28 upstream PyTorch checks: encoder,
nine fixed decoder prompt cases (positive/negative points, boxes, mixed, all
padding and 66-token capacity), inverse resize, and eight orientation round
trips. `reference-golden.npz` retains the synthetic source tensors. Actual
`ai-edge-litert 2.2.0` host CPU execution passed encoder and all nine decoder
cases against those goldens: 19 tensor comparisons, maximum absolute error
`6.771087646484375e-05`, with `atol=rtol=1e-3`. The verified graphs contain
only accepted built-in operators; `SUM` (builtin code 74) was identified in the
pinned LiteRT schema and added to the explicit allowlist. Failed pre-allowlist
conversion attempts remain private and are not the published bundle.

Host peak process RSS after both interpreters executed was 338,452,480 bytes.
Adding all static tensor sizes without reuse gives 5,547,161,189 bytes; adding
known graph and app-side components gives a deliberately pessimistic
5,879,632,093-byte envelope. Neither is a measured Android allocation or an
admissible bound for the existing 256 MiB original/publication pool. Android
LiteRT 2.3.0 CPU/GPU execution, packaged JNI/APK identity and notices, actual
native/delegate working set, cancellation, correction/import/undo, quality and
S23 latency remain unrun. Production admission must stay empty.

To recheck the retained bundle without invoking model inference:

```sh
source /workspace/.vw-tools/cloud-setup/start.sh
python3 tools/assisted-selection/verify_artifacts.py model \
  --root /workspace/assisted-model-delivery-20261019/artifacts/mobile-sam-fixed66 \
  --manifest model-manifest.json \
  --sha256 7ea1a2fe7c979305e65372d85cb3255e49434667cfaa1c636c8d2052518af9f4
python3 -m unittest discover -s tools/assisted-selection/tests -p test_offline_tools.py -v
```

For a fresh host conversion, first use `download_pinned_inputs.py --root
PRIVATE/inputs` in a networked preparation step and install the exact wheels
from `host-wheel-requirements.txt` with hash checking. Execute
`reference_parity.py` and `convert_mobile_sam.py` only in a separate bounded,
network-denied offline process; `offline_exec.py` is the supplied Linux seccomp
launcher. Run `tflite_parity.py` with the resulting graph bundle and private
golden. `make_delivery_manifest.py` checks and binds the retained run after
conversion. The commands and detailed graph contract are in `README.md`; do
not reuse a former artifact hash for a fresh conversion.

For the retained private environment, the distinct preparation and offline
execution commands are:

```sh
source /workspace/.vw-tools/cloud-setup/start.sh
P=/workspace/assisted-model-delivery-20261019
T=tools/assisted-selection
python3 "$T/download_pinned_inputs.py" --root "$P/inputs"
"$P/venv/bin/python" -m pip install --no-index --find-links "$P/wheelhouse" \
  --require-hashes -r "$T/host-wheel-requirements.txt"
env OMP_NUM_THREADS=2 MKL_NUM_THREADS=2 OPENBLAS_NUM_THREADS=2 \
  python3 "$T/offline_exec.py" timeout 1200 "$P/venv/bin/python" \
  "$T/reference_parity.py" --source-root "$P/inputs/MobileSAM-pinned" \
  --checkpoint "$P/inputs/mobile_sam.pt" \
  --output "$P/artifacts/replay-reference.json" \
  --golden "$P/artifacts/replay-reference.npz"
env OMP_NUM_THREADS=2 MKL_NUM_THREADS=2 OPENBLAS_NUM_THREADS=2 \
  python3 "$T/offline_exec.py" timeout 1200 "$P/venv/bin/python" \
  "$T/convert_mobile_sam.py" --source-root "$P/inputs/MobileSAM-pinned" \
  --checkpoint "$P/inputs/mobile_sam.pt" \
  --output "$P/artifacts/mobile-sam-fixed66-replay"
env OMP_NUM_THREADS=2 MKL_NUM_THREADS=2 OPENBLAS_NUM_THREADS=2 \
  python3 "$T/offline_exec.py" timeout 1200 "$P/venv/bin/python" \
  "$T/tflite_parity.py" --bundle "$P/artifacts/mobile-sam-fixed66-replay" \
  --golden "$P/artifacts/replay-reference.npz" \
  --output "$P/artifacts/replay-tflite-parity.json"
```

Choose new private output names for each run; the tools deliberately refuse to
overwrite existing evidence. Capture bounded command output in new private logs.
