# Every packaged file stays read-pinned through validation, execution and recheck.
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
if(-not ('VwDesktopLease' -as [type])) {
Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Collections.Generic;
using System.IO;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;
public sealed class VwDesktopLease : IDisposable {
    private readonly List<SafeFileHandle> handles = new List<SafeFileHandle>();
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    private static extern SafeFileHandle CreateFileW(string path,uint access,uint share,IntPtr security,uint disposition,uint flags,IntPtr template);
    [DllImport("kernel32.dll", SetLastError=true)]
    private static extern bool GetFileInformationByHandleEx(SafeFileHandle handle,int kind,IntPtr data,uint size);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    private static extern uint GetFinalPathNameByHandleW(SafeFileHandle handle,[Out] char[] path,uint count,uint flags);
    public void Pin(string path,bool directory) {
        path=Path.GetFullPath(path);
        uint flags=0x00200000u | (directory ? 0x02000000u : 0x80u);
        var handle=CreateFileW(path,directory?0x80u:0x80000000u,directory?3u:1u,IntPtr.Zero,3,flags,IntPtr.Zero);
        if(handle.IsInvalid){handle.Dispose();throw new InvalidOperationException("Distribution lease unavailable");}
        bool retained=false;
        IntPtr info=Marshal.AllocHGlobal(8);
        try {
            if(!GetFileInformationByHandleEx(handle,9,info,8))throw new InvalidOperationException("Distribution metadata unavailable");
            uint attributes=unchecked((uint)Marshal.ReadInt32(info));
            if((attributes&0x400u)!=0 || ((attributes&0x10u)!=0)!=directory)
                throw new InvalidOperationException("Distribution redirect or type refused");
            var buffer=new char[32768];uint count=GetFinalPathNameByHandleW(handle,buffer,(uint)buffer.Length,0);
            if(count==0 || count>=buffer.Length)throw new InvalidOperationException("Distribution resolution unavailable");
            string actual=new string(buffer,0,checked((int)count));
            if(actual.StartsWith(@"\\?\",StringComparison.Ordinal))actual=actual.Substring(4);
            if(!String.Equals(actual.TrimEnd('\\'),path.TrimEnd('\\'),StringComparison.OrdinalIgnoreCase))
                throw new InvalidOperationException("Distribution resolution changed");
            handles.Add(handle);retained=true;
        } finally {Marshal.FreeHGlobal(info);if(!retained)handle.Dispose();}
    }
    public void Dispose(){for(int i=handles.Count-1;i>=0;i--)handles[i].Dispose();handles.Clear();}
}
'@
}
function Lock-VwDesktopDistribution {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$Directory,[Parameter(Mandatory)][System.Collections.IDictionary]$Inventory)
    $Directory=[IO.Path]::GetFullPath($Directory).TrimEnd('\')
    if($Inventory.Count -lt 4 -or $Inventory.Count -gt 4096){throw 'Distribution inventory count refused.'}
    $lease=[VwDesktopLease]::new()
    $pinned=[Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    try{
        $parents=[Collections.Generic.List[string]]::new()
        $cursor=[IO.DirectoryInfo]::new($Directory)
        while($null -ne $cursor){$parents.Add($cursor.FullName);$cursor=$cursor.Parent}
        for($i=$parents.Count-1;$i -ge 0;$i--){$lease.Pin($parents[$i],$true);[void]$pinned.Add($parents[$i].TrimEnd('\'))}
        foreach($relative in @($Inventory.Keys | Sort-Object)){
            if($relative -notmatch '^[ -~]{1,512}$' -or $relative -match '[\\:<>"|?*]' -or
                $relative.StartsWith('/') -or @($relative.Split('/') | Where-Object{$_ -in @('','.','..') -or $_.EndsWith(' ') -or $_.EndsWith('.')}).Count -gt 0){throw 'Distribution relative path refused.'}
            $path=[IO.Path]::GetFullPath((Join-Path $Directory $relative))
            if(-not $path.StartsWith($Directory+'\',[StringComparison]::OrdinalIgnoreCase)){throw 'Distribution containment refused.'}
            $chain=[Collections.Generic.List[string]]::new();$cursor=[IO.FileInfo]::new($path).Directory
            while($null -ne $cursor -and -not $pinned.Contains($cursor.FullName.TrimEnd('\'))){$chain.Add($cursor.FullName);$cursor=$cursor.Parent}
            for($i=$chain.Count-1;$i -ge 0;$i--){$lease.Pin($chain[$i],$true);[void]$pinned.Add($chain[$i].TrimEnd('\'))}
            $lease.Pin($path,$false)
        }
        $result=$lease;$lease=$null;return $result
    }finally{if($null -ne $lease){$lease.Dispose()}}
}
Export-ModuleMember -Function Lock-VwDesktopDistribution
