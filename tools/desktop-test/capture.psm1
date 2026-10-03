Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
# This module imports only the reviewed Windows Job primitive. It never uses
# Process.BeginOutputReadLine, StreamReader.ReadLine or the helper's output queue.
if(-not ('VwDesktopBoundedBytes' -as [type])) {
Add-Type -TypeDefinition @'
using System;
using System.Diagnostics;
using System.IO;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
public sealed class VwDesktopBoundedBytes {
    private readonly object gate=new object();
    private readonly byte[] stdout,stderr;
    private readonly int limit;
    private int outCount,inCount,total,overflow,readError;
    private Task outTask,inTask;
    public VwDesktopBoundedBytes(int bound) {
        if(bound<64 || bound>1024*1024)throw new ArgumentOutOfRangeException();
        limit=bound;stdout=new byte[bound];stderr=new byte[bound];
    }
    public bool Overflow {get{return Volatile.Read(ref overflow)!=0;}}
    public bool ReadError {get{return Volatile.Read(ref readError)!=0;}}
    public int RetainedBytes {get{lock(gate){return total;}}}
    public bool Complete {get{return outTask!=null && inTask!=null && outTask.IsCompleted && inTask.IsCompleted;}}
    public void Start(Process process) {
        if(outTask!=null)throw new InvalidOperationException();
        outTask=Pump(process.StandardOutput.BaseStream,true);
        inTask=Pump(process.StandardError.BaseStream,false);
    }
    private async Task Pump(Stream stream,bool output) {
        var chunk=new byte[4096];
        try {
            while(true) {
                int count=await stream.ReadAsync(chunk,0,chunk.Length).ConfigureAwait(false);
                if(count==0)return;
                lock(gate) {
                    // Reserve one newline for joining streams. Overflow is a
                    // failure, not a truncated successful transcript. Keep
                    // draining fixed chunks until the owned Job is terminated.
                    if(count>limit-1-total){Volatile.Write(ref overflow,1);continue;}
                    if(overflow!=0)continue;
                    var destination=output?stdout:stderr;
                    int offset=output?outCount:inCount;
                    Buffer.BlockCopy(chunk,0,destination,offset,count);
                    if(output)outCount+=count;else inCount+=count;
                    total+=count;
                }
            }
        } catch {Volatile.Write(ref readError,1);}
        finally {Array.Clear(chunk,0,chunk.Length);}
    }
    public byte[] SuccessfulBytes() {
        if(!Complete || Overflow || ReadError)throw new InvalidOperationException("Capture did not complete");
        lock(gate) {
            // Strict UTF-8 validation has bounded allocation; no raw output is
            // included in an exception or retained process report.
            var utf8=new UTF8Encoding(false,true);
            utf8.GetCharCount(stdout,0,outCount);utf8.GetCharCount(stderr,0,inCount);
            var bytes=new byte[total+1];Buffer.BlockCopy(stdout,0,bytes,0,outCount);
            bytes[outCount]=10;Buffer.BlockCopy(stderr,0,bytes,outCount+1,inCount);
            return bytes;
        }
    }
    public void Clear() {lock(gate){Array.Clear(stdout,0,stdout.Length);Array.Clear(stderr,0,stderr.Length);}}
}
'@
}
function Invoke-VwBoundedDesktopChild {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$Executable,[string[]]$Arguments=@(),
        [Parameter(Mandatory)][string]$WorkingDirectory,
        [ValidateRange(1,300)][int]$TimeoutSeconds=90,
        [ValidateRange(64,1048576)][int]$ByteLimit=1048576,
        [switch]$RetainFailureOutput)
    if(-not $IsWindows -or -not ('VwProcessJobV4' -as [type])){throw 'Owned Windows Job primitive unavailable.'}
    $start=[Diagnostics.ProcessStartInfo]::new()
    $start.FileName=[IO.Path]::GetFullPath($Executable);$start.WorkingDirectory=[IO.Path]::GetFullPath($WorkingDirectory)
    $start.UseShellExecute=$false;$start.CreateNoWindow=$true
    $start.RedirectStandardOutput=$true;$start.RedirectStandardError=$true
    foreach($argument in $Arguments){$start.ArgumentList.Add($argument)}
    foreach($name in @($start.Environment.Keys)){if($name -match '(?i)(TOKEN|SECRET|PASSWORD|API_KEY|PRIVATE_KEY|CREDENTIAL|AUTH_KEY)'){$start.Environment.Remove($name)|Out-Null}}
    $process=[Diagnostics.Process]::new();$process.StartInfo=$start
    $bytes=[VwDesktopBoundedBytes]::new($ByteLimit)
    $job=$null;$started=$false;$assigned=$false;$clean=$false;$complete=$false
    $reason=$null;$exitCode=1;$parentCode=$null;$afterParent=$null;$hadSurvivors=$false
    $watch=[Diagnostics.Stopwatch]::StartNew();$lastProgress=0.0;$output=$null
    try {
        $job=[VwProcessJobV4]::new()
        if(-not $process.Start()){throw 'Packaged process did not start.'}
        $started=$true
        # The wrapper itself is already in the outer Job. This narrower Job is
        # assigned before pipe tasks, with no breakaway permission.
        $job.Assign($process);$assigned=$true;$bytes.Start($process)
        while($true) {
            if($bytes.Overflow){$reason='output_limit';break}
            if($bytes.ReadError){$reason='output_read';break}
            $active=$job.ActiveProcessCount
            if($process.HasExited -and $null -eq $afterParent){$afterParent=$watch.Elapsed.TotalSeconds;$parentCode=$process.ExitCode}
            if($null -ne $afterParent -and $active -eq 0 -and $bytes.Complete){$exitCode=$parentCode;break}
            if($watch.Elapsed.TotalSeconds -ge $TimeoutSeconds){$reason='timeout';$exitCode=124;break}
            if($null -ne $afterParent -and $watch.Elapsed.TotalSeconds-$afterParent -ge 10){$hadSurvivors=$active -gt 0;$reason='parent_exit_drain';break}
            if($watch.Elapsed.TotalSeconds-$lastProgress -ge 15){Write-Host '[desktop-capture] RUNNING';$lastProgress=$watch.Elapsed.TotalSeconds}
            Start-Sleep -Milliseconds 50
        }
    } catch {$reason='capture_failure'}
    finally {
        if($assigned){
            try{
                if($job.ActiveProcessCount -gt 0){$job.Terminate(124)}
                $deadline=[Diagnostics.Stopwatch]::StartNew()
                while(($job.ActiveProcessCount -gt 0 -or -not $bytes.Complete) -and $deadline.Elapsed.TotalSeconds -lt 10){Start-Sleep -Milliseconds 50}
                $clean=$job.ActiveProcessCount -eq 0;$complete=$bytes.Complete
            }catch{$clean=$false}
        }elseif($started){
            try{if(-not $process.HasExited){$process.Kill($true)};$null=$process.WaitForExit(5000)}catch{}
            # Assignment failure never claims descendant cleanup; the outer Job
            # still owns the wrapper and every child for final containment.
        }
        if($null -ne $job){$job.Dispose()}
        if($bytes.Overflow){$reason='output_limit';$exitCode=1}
        # Explicit diagnostics may inspect bounded valid UTF-8 on a nonzero
        # child exit. This never changes its exit status or cleanup checks.
        if($reason -eq $null -and ($exitCode -eq 0 -or $RetainFailureOutput) -and $clean -and $complete){
            try{$output=$bytes.SuccessfulBytes()}catch{$reason='output_encoding';$exitCode=1}
        }
        if(-not $clean -or -not $complete){$reason='cleanup_unconfirmed';$exitCode=125}
        if($null -ne $reason){$exitCode=if($reason -eq 'timeout'){124}else{1}}
        $retained=$bytes.RetainedBytes;$overflow=$bytes.Overflow
        # On normal cleanup both pumps are settled before clearing/disposal.
        # If an OS pipe refuses to settle, the wrapper exits failed and the outer
        # Job reaps it; no late task holds caller files or a workspace writer.
        if($complete){$bytes.Clear()}
        $process.Dispose();$watch.Stop()
    }
    $proof=[ordered]@{phase='desktop-startup';exit_code=$exitCode;parent_exit_code=$parentCode;failure_reason=$reason;
        elapsed_seconds=[Math]::Round($watch.Elapsed.TotalSeconds,3);contained_in_windows_job=$assigned;
        process_tree_cleanup_confirmed=$clean;output_streams_completed=$complete;
        surviving_processes_before_cleanup=$(if($hadSurvivors){@('owned_descendant')}else{@()});compiler_telemetry_cleaned=$false;
        captured_bytes=$retained;capture_byte_limit=$ByteLimit;capture_overflow=$overflow;raw_output_retained=($null -ne $output)}
    return [pscustomobject]@{Proof=$proof;Bytes=$output;ExitCode=$exitCode}
}
Export-ModuleMember -Function Invoke-VwBoundedDesktopChild
