# Ordinary project close result

Ordinary native shutdown errors arrive after worker state destruction. The
wrapper now retains that failure before its closed early return, so another
caller receives the same failure instead of apparent success. Native
closeSession/destroy each run once; a distinct destroy error is suppressed under
the original shutdown error. A failed unadopted-child drain before shutdown
remains retryable. The separate joined giant owner retains its existing precise
retirement/retry behavior. No additional concrete native leak is claimed.

`tools/tests/check_native_project_close.py` compiles the actual unchanged
NativeProject.close body, fields, mapFailure, releaseNative and generic giant
owner, substituting only the foreign handle and unadopted-child drain at their
call boundary. Seven focused Kotlin2.4.20/JUnit cases pass; the preceding
134b421 source fails four cases. Coverage includes sticky mapped shutdown and
destroy failures, suppressed secondary failure, retryable child drain, successful
idempotence, concurrent joined callers and the unchanged giant retirement retry.
This is not a whole-class/new-ABI/real-JNI/app lifecycle execution. Existing
cached jars supply unchanged core types. No blind native destruction retries
were introduced and no hardware acceptance follows from these tests.

Reproduce with the approved setup and existing cached artifacts:

```bash
python3 tools/tests/check_native_project_close.py \
  --tools-root /workspace/.vw-tools \
  --receipt-dir /tmp/vw-close-new-receipt \
  --negative-control-ref 134b421ba5e5732c6cbdb90b1cb73ee831f56793
```

Text receipt is committed in evidence/NATIVE_PROJECT_CLOSE_20261019.json.
Earlier attempts/failures and abandoned draft remain in /tmp; no owner or
worktree has been cleaned. Final coherent shared/app/ABI validation is deferred.
