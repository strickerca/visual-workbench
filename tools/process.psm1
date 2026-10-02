Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if (-not ('VwProcessOutputV4' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.Diagnostics;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text.RegularExpressions;
using System.Threading;

public sealed class VwProcessOutputV4 {
    public readonly ConcurrentQueue<string> Lines = new ConcurrentQueue<string>();
    private readonly string[] redact;
    private int completedStreams;
    public bool IsComplete { get { return Volatile.Read(ref completedStreams) == 2; } }
    public VwProcessOutputV4(string[] redactValues) {
        var variants = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        foreach (string value in redactValues ?? new string[0]) {
            if (String.IsNullOrEmpty(value)) continue;
            variants.Add(value);
            variants.Add(value.Replace("\\", "\\\\"));
            string forward = value.Replace('\\', '/');
            variants.Add(forward);
            string[] parts = forward.Split('/');
            for (int index = 0; index < parts.Length; index++) {
                // Keep the drive colon and path separators used in file URIs.
                if (!(index == 0 && Regex.IsMatch(parts[index], "^[A-Za-z]:$"))) {
                    parts[index] = Uri.EscapeDataString(parts[index]);
                }
            }
            variants.Add(String.Join("/", parts));
        }
        var ordered = new List<string>(variants);
        ordered.Sort((left, right) => right.Length.CompareTo(left.Length));
        redact = ordered.ToArray();
    }
    public void Attach(Process process) {
        process.OutputDataReceived += OnStandardOutput;
        process.ErrorDataReceived += OnStandardError;
    }
    private void OnStandardOutput(object sender, DataReceivedEventArgs args) {
        if (args.Data == null) Interlocked.Increment(ref completedStreams);
        else OnOutput(args.Data);
    }
    private void OnStandardError(object sender, DataReceivedEventArgs args) {
        if (args.Data == null) Interlocked.Increment(ref completedStreams);
        else OnOutput(args.Data);
    }
    private void OnOutput(string line) {
        // Crash reporters sometimes print the complete process environment.
        // Keep the failure, but never retain or display that diagnostic dump.
        if (Regex.IsMatch(line, @"\benv(?:ironment)?\s*:\s*\{", RegexOptions.IgnoreCase)) {
            Lines.Enqueue("[environment diagnostic omitted]");
            return;
        }
        foreach (string value in redact) {
            line = Regex.Replace(line, Regex.Escape(value), "[redacted]", RegexOptions.IgnoreCase | RegexOptions.CultureInvariant);
        }
        Lines.Enqueue(line);
    }
}

public sealed class VwProcessJobV4 : IDisposable {
    private IntPtr handle;
    private const uint KillOnJobClose = 0x00002000;
    [StructLayout(LayoutKind.Sequential)]
    private struct BasicLimitInformation {
        public long PerProcessUserTimeLimit, PerJobUserTimeLimit;
        public uint LimitFlags;
        public UIntPtr MinimumWorkingSetSize, MaximumWorkingSetSize;
        public uint ActiveProcessLimit;
        public UIntPtr Affinity;
        public uint PriorityClass, SchedulingClass;
    }
    [StructLayout(LayoutKind.Sequential)]
    private struct IoCounters {
        public ulong ReadOperationCount, WriteOperationCount, OtherOperationCount;
        public ulong ReadTransferCount, WriteTransferCount, OtherTransferCount;
    }
    [StructLayout(LayoutKind.Sequential)]
    private struct ExtendedLimitInformation {
        public BasicLimitInformation Basic;
        public IoCounters Io;
        public UIntPtr ProcessMemoryLimit, JobMemoryLimit, PeakProcessMemoryUsed, PeakJobMemoryUsed;
    }
    [StructLayout(LayoutKind.Sequential)]
    private struct BasicAccountingInformation {
        public long TotalUserTime, TotalKernelTime, ThisPeriodTotalUserTime, ThisPeriodTotalKernelTime;
        public uint TotalPageFaultCount, TotalProcesses, ActiveProcesses, TotalTerminatedProcesses;
    }
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern IntPtr CreateJobObject(IntPtr attributes, string name);
    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool SetInformationJobObject(IntPtr job, int informationClass,
        ref ExtendedLimitInformation information, uint length);
    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool QueryInformationJobObject(IntPtr job, int informationClass,
        out BasicAccountingInformation information, uint length, IntPtr returnLength);
    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool QueryInformationJobObject(IntPtr job, int informationClass,
        IntPtr information, uint length, IntPtr returnLength);
    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);
    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool TerminateJobObject(IntPtr job, uint exitCode);
    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool CloseHandle(IntPtr handle);

    public VwProcessJobV4() {
        handle = CreateJobObject(IntPtr.Zero, null);
        if (handle == IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error());
        var limits = new ExtendedLimitInformation();
        limits.Basic.LimitFlags = KillOnJobClose;
        if (!SetInformationJobObject(handle, 9, ref limits,
                (uint)Marshal.SizeOf(typeof(ExtendedLimitInformation)))) {
            int error = Marshal.GetLastWin32Error();
            Dispose();
            throw new Win32Exception(error);
        }
    }
    public void Assign(Process process) {
        if (!AssignProcessToJobObject(handle, process.Handle))
            throw new Win32Exception(Marshal.GetLastWin32Error());
    }
    public uint ActiveProcessCount {
        get {
            BasicAccountingInformation information;
            if (!QueryInformationJobObject(handle, 1, out information,
                    (uint)Marshal.SizeOf(typeof(BasicAccountingInformation)), IntPtr.Zero))
                throw new Win32Exception(Marshal.GetLastWin32Error());
            return information.ActiveProcesses;
        }
    }
    public void Terminate(uint exitCode) {
        if (!TerminateJobObject(handle, exitCode))
            throw new Win32Exception(Marshal.GetLastWin32Error());
    }
    public void Dispose() {
        IntPtr closing = Interlocked.Exchange(ref handle, IntPtr.Zero);
        if (closing != IntPtr.Zero) CloseHandle(closing);
    }
    public int[] ActiveProcessIds {
        get {
            int capacity = 4096;
            int size = 8 + capacity * IntPtr.Size;
            IntPtr buffer = Marshal.AllocHGlobal(size);
            try {
                if (!QueryInformationJobObject(handle, 3, buffer, (uint)size, IntPtr.Zero))
                    throw new Win32Exception(Marshal.GetLastWin32Error());
                int count = Marshal.ReadInt32(buffer, 4);
                if (count < 0 || count > capacity) throw new InvalidOperationException("Job census exceeds bound");
                int[] ids = new int[count];
                for (int index = 0; index < count; index++)
                    ids[index] = checked((int)Marshal.ReadIntPtr(buffer, 8 + index * IntPtr.Size).ToInt64());
                return ids;
            } finally { Marshal.FreeHGlobal(buffer); }
        }
    }
}
'@
}

function ConvertTo-VwNativeArgument {
    param([AllowEmptyString()][string]$Value)
    if ($Value -notmatch '[\s"]' -and $Value.Length -gt 0) { return $Value }
    # Windows CommandLineToArgvW quoting: double backslashes preceding a quote,
    # and double trailing backslashes before the closing quote.
    $quoted = [regex]::Replace($Value, '(\\*)"', '$1$1\"')
    $quoted = [regex]::Replace($quoted, '(\\+)$', '$1$1')
    return '"' + $quoted + '"'
}

function Invoke-VwProcess {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][string]$FilePath,
        [string[]]$ArgumentList = @(),
        [Parameter(Mandatory = $true)][string]$WorkingDirectory,
        [string]$Phase = 'command',
        [ValidateRange(1, 86400)][int]$TimeoutSeconds = 600,
        [ValidateRange(1, 60)][int]$ParentExitGraceSeconds = 5,
        [string[]]$RedactValues = @(),
        [switch]$Capture,
        [switch]$SensitiveCapture,
        [switch]$CleanCompilerTelemetry
    )
    $commandInfo = Get-Command $FilePath -ErrorAction SilentlyContinue
    if (-not $commandInfo -and -not (Test-Path -LiteralPath $FilePath -PathType Leaf)) {
        throw "[$Phase] Required command is unavailable: $FilePath"
    }
    $resolvedCommand = if ($commandInfo) { $commandInfo.Source } else { (Resolve-Path -LiteralPath $FilePath).Path }
    $arguments = ($ArgumentList | ForEach-Object { ConvertTo-VwNativeArgument $_ }) -join ' '
    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $resolvedCommand
    $startInfo.Arguments = $arguments
    if ([IO.Path]::GetExtension($resolvedCommand) -in @('.cmd', '.bat')) {
        # Only fixed build-task arguments reach cmd. Reject shell metacharacters;
        # do not interpolate arbitrary command text into a command-shell script.
        foreach ($argument in @($resolvedCommand) + $ArgumentList) {
            if ($argument -match '[&|<>^%!\r\n]') { throw "[$Phase] Unsafe batch-file argument" }
        }
        $startInfo.FileName = $env:ComSpec
        $startInfo.Arguments = '/d /s /c ""' + $resolvedCommand + '" ' + $arguments + '"'
    }
    $startInfo.WorkingDirectory = (Resolve-Path -LiteralPath $WorkingDirectory).Path
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    # The public dependency repositories used by this build require no tokens.
    # Remove unrelated inherited credentials before any compiler/helper runs;
    # also redact their original values if another tool prints them indirectly.
    $effectiveRedactions = New-Object System.Collections.Generic.List[string]
    if (-not $SensitiveCapture) {
        # Normal diagnostics never need local account/workspace identities.
        # SensitiveCapture keeps raw compiler environment paths in memory so
        # callers can consume valid JSON; those lines never enter a log or UI.
        if ($env:USERPROFILE) { $effectiveRedactions.Add($env:USERPROFILE) }
        $effectiveRedactions.Add($startInfo.WorkingDirectory)
    }
    foreach ($value in $RedactValues) { $effectiveRedactions.Add($value) }
    foreach ($name in @($startInfo.EnvironmentVariables.Keys)) {
        if ($name -match '(?i)(TOKEN|SECRET|PASSWORD|API_KEY|PRIVATE_KEY|CREDENTIAL|AUTH_KEY)') {
            $value = $startInfo.EnvironmentVariables[$name]
            if ($value) { $effectiveRedactions.Add($value) }
            $startInfo.EnvironmentVariables.Remove($name)
        }
    }
    $process = New-Object System.Diagnostics.Process
    $process.StartInfo = $startInfo
    $output = New-Object VwProcessOutputV4 -ArgumentList (, $effectiveRedactions.ToArray())
    $output.Attach($process)
    $lines = New-Object System.Collections.Generic.List[string]
    $logDirectory = Join-Path ([IO.Path]::GetTempPath()) 'VisualWorkbench-setup-text-logs'
    [IO.Directory]::CreateDirectory($logDirectory) | Out-Null
    $logName = [regex]::Replace($Phase, '[^a-zA-Z0-9_.-]', '-') + '-' + [guid]::NewGuid().ToString('N') + '.log'
    $logPath = Join-Path $logDirectory $logName
    $writer = New-Object System.IO.StreamWriter -ArgumentList @($logPath, $false, (New-Object System.Text.UTF8Encoding -ArgumentList $false))
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $lastProgress = 0
    $lastMemorySample = -2
    $peakWorkingSet = [long]0
    $started = $false
    $job = $null
    $jobAssigned = $false
    $cleanupConfirmed = $false
    $parentExitedAt = $null
    $parentExitCode = $null
    $failureReason = $null
    $survivingProcesses = @()
    $compilerTelemetryCleaned = $false
    $exitCode = 1
    if ($SensitiveCapture) { $Capture = $true; $writer.WriteLine('[sensitive command output retained in memory only]') }
    Write-Host "[$Phase] START (limit ${TimeoutSeconds}s)"
    try {
        $job = New-Object VwProcessJobV4
        if (-not $process.Start()) { throw "[$Phase] Could not start process" }
        $started = $true
        # Assign immediately, before reading output or doing any other work.
        # Failure to contain the process is a failure of the bounded runner.
        $job.Assign($process)
        $jobAssigned = $true
        $process.BeginOutputReadLine()
        $process.BeginErrorReadLine()
        while ($true) {
            $line = $null
            $drainedLines = 0
            # A continuously writing descendant must not starve the deadline.
            while ($drainedLines -lt 1000 -and $output.Lines.TryDequeue([ref]$line)) {
                if (-not $SensitiveCapture) { $writer.WriteLine($line) }
                if ($Capture) { $lines.Add($line) } else { Write-Host $line }
                $drainedLines++
            }
            $writer.Flush()
            $parentExited = $process.HasExited
            $activeProcesses = $job.ActiveProcessCount
            if ($parentExited -and $null -eq $parentExitedAt) {
                $parentExitedAt = $watch.Elapsed.TotalSeconds
                $parentExitCode = $process.ExitCode
            }
            if ($parentExited -and $activeProcesses -eq 0 -and $output.IsComplete -and $output.Lines.IsEmpty) {
                $exitCode = $parentExitCode
                break
            }
            if ($watch.Elapsed.TotalSeconds -ge $TimeoutSeconds) {
                $failureReason = 'overall deadline exceeded, including output drain'
                Write-Host "[$Phase] TIMEOUT: stopping the contained process tree; inspect the phase log before rerunning."
                $exitCode = 124
                break
            }
            # Launchers can exit just before a single-use Gradle child finishes.
            # Permit a brief shutdown, bounded by the same overall deadline.
            if ($parentExited -and $watch.Elapsed.TotalSeconds - $parentExitedAt -ge $ParentExitGraceSeconds) {
                $failureReason = if ($activeProcesses -gt 0) { 'descendants survived parent exit' } else { 'output streams did not reach EOF after parent exit' }
                $survivingProcesses = @($job.ActiveProcessIds | ForEach-Object {
                    $ownedProcess = Get-Process -Id $_ -ErrorAction SilentlyContinue
                    if ($ownedProcess) { [ordered]@{ pid = $ownedProcess.Id; name = $ownedProcess.ProcessName } }
                })
                # MSVC's telemetry uploader can remain idle after a successful
                # compile. This explicit compiler-only exception still closes
                # the exact job and confirms cleanup; other survivors fail.
                $onlyCompilerTelemetry = $CleanCompilerTelemetry -and $parentExitCode -eq 0 -and $output.IsComplete -and $survivingProcesses.Count -gt 0
                foreach ($survivor in $survivingProcesses) {
                    $ownedProcess = Get-Process -Id $survivor.pid -ErrorAction SilentlyContinue
                    if (-not $ownedProcess -or $ownedProcess.ProcessName -ne 'vctip' -or
                        $ownedProcess.Path -notmatch '\\Microsoft Visual Studio\\[^\\]+\\[^\\]+\\VC\\Tools\\MSVC\\[^\\]+\\bin\\Hostx64\\x64\\vctip\.exe$' -or
                        $ownedProcess.MainModule.FileVersionInfo.CompanyName -ne 'Microsoft Corporation') { $onlyCompilerTelemetry = $false }
                }
                if ($onlyCompilerTelemetry) {
                    Write-Host "[$Phase] CLEANUP: successful compiler left only its verified MSVC telemetry helper."
                    $compilerTelemetryCleaned = $true
                    $failureReason = $null
                    $exitCode = $parentExitCode
                    break
                }
                Write-Host "[$Phase] FAIL: $failureReason; stopping the contained process tree."
                $exitCode = 1
                break
            }
            if ($watch.Elapsed.TotalSeconds - $lastMemorySample -ge 2) {
                # Exact job membership avoids unrelated processes and survives
                # launcher exit; process handles avoid WMI census delays.
                $workingSet = [long]0
                foreach ($ownedId in $job.ActiveProcessIds) {
                    $ownedProcess = Get-Process -Id $ownedId -ErrorAction SilentlyContinue
                    if ($ownedProcess) { $workingSet += [long]$ownedProcess.WorkingSet64 }
                }
                if ($workingSet -gt $peakWorkingSet) { $peakWorkingSet = $workingSet }
                $lastMemorySample = $watch.Elapsed.TotalSeconds
            }
            if ($watch.Elapsed.TotalSeconds - $lastProgress -ge 15) {
                Write-Host "[$Phase] RUNNING $([int]$watch.Elapsed.TotalSeconds)s"
                $lastProgress = $watch.Elapsed.TotalSeconds
            }
            Start-Sleep -Milliseconds 100
        }
    } finally {
        # Job membership outlives the launcher. Closing this job kills only its
        # processes, even when the original process has already exited.
        if ($jobAssigned) {
            try {
                if ($job.ActiveProcessCount -gt 0) { $job.Terminate(124) }
                $cleanupWatch = [Diagnostics.Stopwatch]::StartNew()
                while ($job.ActiveProcessCount -gt 0 -and $cleanupWatch.Elapsed.TotalSeconds -lt 10) {
                    Start-Sleep -Milliseconds 100
                }
                $cleanupConfirmed = $job.ActiveProcessCount -eq 0
                $cleanupWatch.Stop()
            } catch { Write-Warning "[$Phase] Contained process cleanup could not be confirmed" }
        } elseif ($started -and -not $process.HasExited) {
            # Assignment failed before containment: stop the exact process tree
            # started by this invocation, then propagate the original failure.
            & (Join-Path $env:SystemRoot 'System32\taskkill.exe') /PID $process.Id /T /F *> $null
        }
        if ($job) { $job.Dispose() }
        if ($jobAssigned -and -not $cleanupConfirmed) {
            $exitCode = 125
            $failureReason = 'contained process cleanup could not be confirmed'
            Write-Warning "[$Phase] Process cleanup could not be confirmed"
        }
        # Never call parameterless WaitForExit(): inherited pipes can remain
        # open after the launcher exits. Cancellation/disposal stays bounded.
        if ($started -and -not $output.IsComplete) {
            try { $process.CancelOutputRead() } catch { }
            try { $process.CancelErrorRead() } catch { }
        }
        $line = $null
        while ($output.Lines.TryDequeue([ref]$line)) {
            if (-not $SensitiveCapture) { $writer.WriteLine($line) }
            if ($Capture) { $lines.Add($line) } else { Write-Host $line }
        }
        $writer.Dispose()
        $process.Dispose()
        $watch.Stop()
    }
    Write-Host "[$Phase] END exit=$exitCode elapsed=$([math]::Round($watch.Elapsed.TotalSeconds, 1))s peak-tree-working-set=$([math]::Round($peakWorkingSet / 1MB, 1))MiB log=$logName"
    $receipt = [ordered]@{ phase = $Phase; exit_code = $exitCode; parent_exit_code = $parentExitCode; failure_reason = $failureReason; elapsed_seconds = [math]::Round($watch.Elapsed.TotalSeconds, 3); sampled_peak_tree_working_set_bytes = $peakWorkingSet; sampling_interval_seconds = 2; timeout_seconds = $TimeoutSeconds; contained_in_windows_job = $jobAssigned; process_tree_cleanup_confirmed = $cleanupConfirmed; output_streams_completed = $output.IsComplete; log_file = $logName }
    $receipt.surviving_processes_before_cleanup = $survivingProcesses
    $receipt.compiler_telemetry_cleaned = $compilerTelemetryCleaned
    $receipt.parent_exit_grace_seconds = $ParentExitGraceSeconds
    $receipt | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath ($logPath + '.json') -Encoding UTF8
    return [pscustomobject]@{ ExitCode = $exitCode; Lines = $lines.ToArray(); LogPath = $logPath; ElapsedSeconds = $watch.Elapsed.TotalSeconds; PeakTreeWorkingSetBytes = $peakWorkingSet }
}

Export-ModuleMember -Function Invoke-VwProcess
