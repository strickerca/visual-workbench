# Pure closed-line observation; no device/window/process actions or settlement.
function Get-M4ReceiverObservation([string]$Line) {
    if($null -eq $Line -or $Line.Length -gt 256 -or $Line -cnotmatch '^REMOTE_RECEIVER_COUNTS:down=([0-9]{1,5});update=([0-9]{1,5});up=([0-9]{1,5});mouse_down=([0-9]{1,5});mouse_up=([0-9]{1,5});wheel=([0-9]{1,5});journal_bytes=([0-9]{1,8}|unknown)$'){return $null}
    $counts=@(1..6|ForEach-Object{[int]$Matches[$_]})
    if(($counts|Measure-Object -Sum).Sum -gt 20000){return $null}
    $bytes=if($Matches[7] -ceq 'unknown'){$null}else{[int]$Matches[7]}
    if($null -ne $bytes -and $bytes -gt 16777216){return $null}
    return [pscustomobject]@{schema=1;counts=$counts;journal_handle_bytes=$bytes;acceptance=$false}
}
