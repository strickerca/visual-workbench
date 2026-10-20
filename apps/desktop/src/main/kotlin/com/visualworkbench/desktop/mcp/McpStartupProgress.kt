package com.visualworkbench.desktop.mcp

/** service_open covers the complete native service factory, not an inferred
 * individual native substage. Refusal categories remain a separate contract. */
internal enum class McpStartupPhase {
    unknown, bundle_parents, bundle_manifest, bundle_extract, bundle_markers,
    bundle_inventory, bundle_verify, service_open, ready,
}

/** Diagnostic counters only: no paths, identities, exception text or authority.
 * The application-owned producer updates this record; a cancelled observer
 * neither cancels that producer nor marks its resources retired. */
internal class McpStartupProgress(private val nowNanos: () -> Long = System::nanoTime) {
    private val lock = Any()
    private var started: Long? = null
    private var phaseStarted = 0L
    private var finished: Long? = null
    private var phase = McpStartupPhase.unknown
    private var attempt = 0
    private val durations = LongArray(McpStartupPhase.entries.size)
    private var files = 0
    private var bytes = 0L
    private var verifiedFiles = 0
    private var verifiedBytes = 0L

    fun begin() = synchronized(lock) {
        val now = nowNanos()
        started = now; phaseStarted = now; finished = null
        phase = McpStartupPhase.unknown
        attempt = (attempt + 1).coerceAtMost(65_535)
        durations.fill(0)
        files = 0; bytes = 0; verifiedFiles = 0; verifiedBytes = 0
    }
    fun phase(value: McpFailureStage) = phase(when (value) {
        McpFailureStage.bundle_parents -> McpStartupPhase.bundle_parents
        McpFailureStage.bundle_manifest -> McpStartupPhase.bundle_manifest
        McpFailureStage.bundle_extract -> McpStartupPhase.bundle_extract
        McpFailureStage.bundle_markers -> McpStartupPhase.bundle_markers
        McpFailureStage.bundle_inventory -> McpStartupPhase.bundle_inventory
        McpFailureStage.bundle_verify -> McpStartupPhase.bundle_verify
        else -> McpStartupPhase.unknown
    })
    fun phase(value: McpStartupPhase) = synchronized(lock) {
        if (started == null || finished != null) return@synchronized
        val now = nowNanos()
        durations[phase.ordinal] = (durations[phase.ordinal] + elapsed(phaseStarted, now)).coerceAtMost(MAX_NANOS)
        phase = value; phaseStarted = now
    }
    fun inventory(fileCount: Int, totalBytes: Long) = synchronized(lock) {
        files = fileCount.coerceIn(0, 20_000)
        bytes = totalBytes.coerceIn(0, MAX_BYTES)
    }
    fun verifiedFile(size: Long) = synchronized(lock) {
        verifiedFiles = (verifiedFiles + 1).coerceAtMost(files)
        verifiedBytes = (verifiedBytes + size.coerceIn(0, MAX_BYTES)).coerceAtMost(bytes)
    }
    fun finish() = synchronized(lock) { if (started != null && finished == null) finished = nowNanos() }

    fun snapshot(): Snapshot = synchronized(lock) {
        val start = started
        val now = finished ?: nowNanos()
        val total = if (start == null) 0L else elapsed(start, now)
        val phases = durations.copyOf()
        if (start != null) phases[phase.ordinal] = (phases[phase.ordinal] + elapsed(phaseStarted, now)).coerceAtMost(total)
        Snapshot(attempt, phase, total / 1_000_000, files, bytes, verifiedFiles, verifiedBytes,
            phases.map { it.coerceAtMost(total) / 1_000_000 })
    }

    internal data class Snapshot(
        val attempt: Int, val phase: McpStartupPhase, val elapsedMillis: Long,
        val files: Int, val bytes: Long, val verifiedFiles: Int, val verifiedBytes: Long,
        val phaseMillis: List<Long>,
    ) {
        fun fields(): String = "attempt=$attempt phase=${phase.name} start_ms=$elapsedMillis files=$files bytes=$bytes " +
            "hashed_files=$verifiedFiles hashed_bytes=$verifiedBytes " +
            McpStartupPhase.entries.joinToString(" ") { "${it.name}_ms=${phaseMillis[it.ordinal]}" }
    }
    companion object {
        private const val MAX_NANOS = 3_600_000_000_000L
        private const val MAX_BYTES = 1_073_741_824L
        private fun elapsed(start: Long, end: Long): Long = (end - start).coerceIn(0, MAX_NANOS)
        internal fun elapsedMillis(start: Long, end: Long): Long = elapsed(start, end) / 1_000_000
    }
}
