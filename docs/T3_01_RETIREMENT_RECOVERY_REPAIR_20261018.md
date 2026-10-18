# Capture retirement fixture recovery repair

Combined base: `70475b818b2ca5bf02b4a3539fb038c864410d83`.
One-file source repair: `350f492b62627fac36b36e83715b15441918d435` on
`task/t3.01-retirement-recovery-test-20261018`.

`CaptureCallerRetirementTest` asserted Throwable identity after crossing
`withContext(NonCancellable)`. Normal coroutine stack-trace recovery can report
a copied exception whose cause is the original. The repaired assertion requires
the same concrete exception class and message, plus identity of the exact
original or its recovered cause. It separately queries the synchronous retained
release action and requires the original object there. Both picker/foreground
failure cases still retry twice, require one destructor attempt and no parent
closure, and forbid foreground retirement after a picker refusal. Production
source and test/recovery configuration are unchanged.

Focused execution used six exact repository sources and an external runner,
cached Kotlin 2.4.0, JDK 21, repository-pinned coroutines 1.9.0, JUnit 4.13.2 and
Java `-ea`. In the JUnit JVM, the runner requires actual assertion/debug/recovery
flags to be true and both coroutine system properties to be absent. It compiles
no projected API or substitute implementation and acquires the shared build
lock; compile/test processes have 60/25-second bounds. No new dependency was
downloaded or installed. This is a direct focused execution, not a fresh Gradle
application build.

- Unmodified fixture: seven tests, one expected failure at the original line 33.
- Repaired fixture: seven tests, zero failures or ignores: all three retirement
  tests and all four adjacent admission tests pass under the same normal policy.
- Adjacent direct-destructor/project ownership assertions were reviewed together.
  Their source is unchanged. The composed receipt already records normal-policy
  passes for all three delivery and 31 picker cases; those 34 cases were
  not rerun here and are not counted in the seven fresh executions.

Exact commands, tool/source/output hashes, runner/driver source, original failure
and repaired output are retained in the
[JSON receipt](evidence/T3_01_CAPTURE_RETIREMENT_RECOVERY_20261018.json) and
[excerpts](evidence/T3_01_CAPTURE_RETIREMENT_RECOVERY_20261018_EXCERPTS.txt).
The first external runner compilation rejected a generic method reference; the
runner alone was corrected before baseline reproduction. Its failure remains
in the owned temporary evidence directory and is recorded in the receipt.

No entire combined build, native compilation, binding generation, Android lint,
device or Windows runtime check was repeated. Existing capture/DPI/exclusion,
hotkey, GPU/physical input/performance and owner acceptance gates remain open.
The coordinator retains the separate Android correction and affected composed
caller/test/lint batch; this repair does not declare the whole phase passed.

Next: independent Astra review and coordinator composition, small fixture repair,
requested `gpt-6.1-sol` / `xhigh`. This session's actual model/effort is not exposed;
no settings were changed. No main merge or promotion is performed by this lane.
