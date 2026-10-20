using System;
using System.ComponentModel;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.Win32.SafeHandles;

// This owner never sets KILL_ON_JOB_CLOSE. A lost parent must not destroy a
// destination whose remote input release is still Pending.
public sealed class M4FrameChild : IDisposable {
    [StructLayout(LayoutKind.Sequential)] private struct Startup {
        public uint cb; public IntPtr reserved, desktop, title;
        public uint x,y,cx,cy,xchars,ychars,fill,flags;
        public ushort show,reservedSize; public IntPtr reservedBytes,input,output,error;
    }
    [StructLayout(LayoutKind.Sequential)] private struct StartupEx { public Startup start; public IntPtr attributes; }
    [StructLayout(LayoutKind.Sequential)] private struct ProcessInfo { public IntPtr process,thread; public uint pid,tid; }
    [StructLayout(LayoutKind.Sequential)] private struct Security { public uint size; public IntPtr descriptor; [MarshalAs(UnmanagedType.Bool)] public bool inherit; }
    [StructLayout(LayoutKind.Sequential)] private struct Accounting {
        public long user,kernel,periodUser,periodKernel; public uint faults,total,active,terminated;
    }
    [StructLayout(LayoutKind.Sequential)] private struct FileTime { public uint low,high; public ulong Value { get { return ((ulong)high<<32)|low; } } }
    [StructLayout(LayoutKind.Sequential)] private struct Point { public int x,y; }
    private delegate bool EnumWindow(IntPtr window, IntPtr data);
    [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)] private static extern IntPtr CreateJobObjectW(IntPtr security,string name);
    [DllImport("kernel32.dll",SetLastError=true)] private static extern bool AssignProcessToJobObject(IntPtr job,IntPtr process);
    [DllImport("kernel32.dll",SetLastError=true)] private static extern bool IsProcessInJob(IntPtr process,IntPtr job,out bool present);
    [DllImport("kernel32.dll",SetLastError=true)] private static extern bool QueryInformationJobObject(IntPtr job,int kind,out Accounting accounting,uint size,IntPtr returned);
    [DllImport("kernel32.dll",SetLastError=true)] private static extern bool TerminateJobObject(IntPtr job,uint code);
    [DllImport("kernel32.dll",SetLastError=true)] private static extern bool TerminateProcess(IntPtr process,uint code);
    [DllImport("kernel32.dll",SetLastError=true)] private static extern bool CloseHandle(IntPtr handle);
    [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)] private static extern bool CreateProcessW(string image,StringBuilder command,IntPtr processSecurity,IntPtr threadSecurity,bool inherit,uint flags,IntPtr environment,string directory,ref StartupEx startup,out ProcessInfo info);
    [DllImport("kernel32.dll",SetLastError=true)] private static extern uint ResumeThread(IntPtr thread);
    [DllImport("kernel32.dll",SetLastError=true)] private static extern bool GetProcessTimes(IntPtr process,out FileTime created,out FileTime exited,out FileTime kernel,out FileTime user);
    [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)] private static extern bool QueryFullProcessImageNameW(IntPtr process,uint flags,StringBuilder path,ref uint length);
    [DllImport("kernel32.dll",SetLastError=true)] private static extern uint WaitForSingleObject(IntPtr handle,uint milliseconds);
    [DllImport("kernel32.dll",SetLastError=true)] private static extern bool GetExitCodeProcess(IntPtr process,out uint code);
    [DllImport("kernel32.dll",SetLastError=true)] private static extern bool QueryPerformanceCounter(out long value);
    [DllImport("kernel32.dll",SetLastError=true)] private static extern bool QueryPerformanceFrequency(out long value);
    [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)] private static extern bool CreateDirectoryW(string path,IntPtr security);
    [DllImport("kernel32.dll",SetLastError=true)] private static extern bool CreatePipe(out IntPtr read,out IntPtr write,ref Security security,uint size);
    [DllImport("kernel32.dll",SetLastError=true)] private static extern bool SetHandleInformation(IntPtr handle,uint mask,uint flags);
    [DllImport("kernel32.dll",SetLastError=true)] private static extern bool InitializeProcThreadAttributeList(IntPtr list,int count,uint flags,ref IntPtr size);
    [DllImport("kernel32.dll",SetLastError=true)] private static extern bool UpdateProcThreadAttribute(IntPtr list,uint flags,IntPtr attribute,IntPtr value,IntPtr size,IntPtr previous,IntPtr returned);
    [DllImport("kernel32.dll")] private static extern void DeleteProcThreadAttributeList(IntPtr list);
    [DllImport("user32.dll")] private static extern bool EnumWindows(EnumWindow callback,IntPtr data);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr window,out uint pid);
    [DllImport("user32.dll")] private static extern IntPtr GetAncestor(IntPtr window,uint flags);
    [DllImport("user32.dll")] private static extern IntPtr GetWindow(IntPtr window,uint command);
    [DllImport("user32.dll")] private static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] private static extern bool IsWindow(IntPtr window);
    [DllImport("user32.dll")] private static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll",SetLastError=true)] private static extern bool GetCursorPos(out Point point);
    [DllImport("user32.dll")] private static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")] private static extern bool ShowWindowAsync(IntPtr window,int command);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] private static extern int GetWindowTextW(IntPtr window,StringBuilder text,int length);
    [DllImport("user32.dll",SetLastError=true)] private static extern bool SetProcessDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll")] private static extern IntPtr GetThreadDpiAwarenessContext();
    [DllImport("user32.dll")] private static extern bool AreDpiAwarenessContextsEqual(IntPtr first,IntPtr second);
    private IntPtr job, processHandle, threadHandle;
    private uint pid,initialThread;
    private ulong created;
    private string image;
    private IntPtr window;
    private uint windowThread;
    private Task<string> stdout,stderr;
    private bool assigned,disposed;
    public Process Process { get; private set; }
    public string StartupError { get; private set; }
    public bool SuspendedBeforeAssignment { get; private set; }
    public bool ExactImageVerifiedBeforeResume { get; private set; }
    public bool Resumed { get; private set; }
    public uint ProcessId { get { return pid; } }
    public ulong Created { get { return created; } }
    public uint InitialThreadId { get { return initialThread; } }
    public long Window { get { return window.ToInt64(); } }
    public uint WindowThreadId { get { return windowThread; } }
    public bool OutputTooLarge { get; private set; }
    public bool IoJoined { get { return (stdout==null||stdout.IsCompleted)&&(stderr==null||stderr.IsCompleted); } }
    public string StandardOutput { get { if(!IoJoined)throw new InvalidOperationException("IO Pending"); if(stderr!=null)stderr.GetAwaiter().GetResult(); return stdout==null?"":stdout.GetAwaiter().GetResult(); } }
    public uint ExitCode { get { uint code; if(!HasExited||!GetExitCodeProcess(processHandle,out code))throw new InvalidOperationException("Exit Pending"); return code; } }
    public bool HasExited { get { if(processHandle==IntPtr.Zero)return true; uint result=WaitForSingleObject(processHandle,0); if(result==0xffffffff)throw new Win32Exception(Marshal.GetLastWin32Error()); return result==0; } }
    public uint ActiveProcesses { get { if(job==IntPtr.Zero)return 0; Accounting value; if(!QueryInformationJobObject(job,1,out value,(uint)Marshal.SizeOf(typeof(Accounting)),IntPtr.Zero))throw new Win32Exception(Marshal.GetLastWin32Error()); return value.active; } }
    public bool IsSettled { get { return HasExited&&ActiveProcesses==0&&IoJoined; } }
    public static void InitializeDpi() {
        IntPtr pmv2=new IntPtr(-4);
        if(!SetProcessDpiAwarenessContext(pmv2)&&!AreDpiAwarenessContextsEqual(GetThreadDpiAwarenessContext(),pmv2))throw new InvalidOperationException("Per-monitor-v2 DPI required");
    }
    public static ulong Clock100ns() {
        long counter,frequency;if(!QueryPerformanceCounter(out counter)||!QueryPerformanceFrequency(out frequency)||counter<0||frequency<=0)throw new InvalidOperationException("Native QPC unavailable");
        // Match Rust clock_ns() floor followed by clock_100ns() div_ceil(100).
        decimal ns=Decimal.Truncate((decimal)counter*1000000000m/frequency);
        return checked((ulong)Decimal.Ceiling(ns/100m));
    }
    public static string NewNativeId() {
        var bytes=new byte[16];RandomNumberGenerator.Fill(bytes);long milliseconds=DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();
        if(milliseconds<0||milliseconds>=281474976710656L)throw new InvalidOperationException("UUIDv7 clock bound");
        for(int index=5;index>=0;index--){bytes[index]=(byte)milliseconds;milliseconds>>=8;}
        bytes[6]=(byte)((bytes[6]&15)|0x70);bytes[8]=(byte)((bytes[8]&63)|0x80);
        string hex=Convert.ToHexString(bytes).ToLowerInvariant();return hex.Substring(0,8)+"-"+hex.Substring(8,4)+"-"+hex.Substring(12,4)+"-"+hex.Substring(16,4)+"-"+hex.Substring(20,12);
    }
    public static int[] CursorPosition() {Point point;if(!GetCursorPos(out point))throw new Win32Exception(Marshal.GetLastWin32Error());return new[]{point.x,point.y};}
    public static void CreateOwnedDirectory(string path) {if(!CreateDirectoryW(path,IntPtr.Zero))throw new Win32Exception(Marshal.GetLastWin32Error());}
    private static string Quote(string value) {
        if(value.IndexOf('\0')>=0)throw new ArgumentException("NUL argument");
        var result=new StringBuilder("\"");int slashes=0;
        foreach(char c in value){if(c=='\\'){slashes++;continue;}if(c=='\"'){result.Append('\\',slashes*2+1);result.Append(c);}else{result.Append('\\',slashes);result.Append(c);}slashes=0;}
        result.Append('\\',slashes*2);return result.Append('"').ToString();
    }
    private Task<string> Drain(ref IntPtr handle,int limit) {
        var safe=new SafeFileHandle(handle,true);handle=IntPtr.Zero;FileStream stream=null;
        try{
            stream=new FileStream(safe,FileAccess.Read,4096,false);
            return Task.Run(()=>{using(stream)using(var reader=new StreamReader(stream,new UTF8Encoding(false,true),false,4096)){var output=new StringBuilder();var buffer=new char[4096];int count;while((count=reader.Read(buffer,0,buffer.Length))>0){if(output.Length+count<=limit)output.Append(buffer,0,count);else OutputTooLarge=true;}return output.ToString();}});
        }catch{if(stream!=null)stream.Dispose();else safe.Dispose();throw;}
    }
    // Returns a retained owner even when startup fails after CreateProcess.
    public static M4FrameChild Launch(string exactImage,string[] arguments,bool helper) {
        var owner=new M4FrameChild();owner.image=Path.GetFullPath(exactImage);
        IntPtr inRead=IntPtr.Zero,inWrite=IntPtr.Zero,outRead=IntPtr.Zero,outWrite=IntPtr.Zero,errRead=IntPtr.Zero,errWrite=IntPtr.Zero,list=IntPtr.Zero,handles=IntPtr.Zero;
        bool listReady=false;
        try {
            owner.job=CreateJobObjectW(IntPtr.Zero,null);if(owner.job==IntPtr.Zero)throw new Win32Exception(Marshal.GetLastWin32Error());
            var startup=new StartupEx();startup.start.cb=(uint)Marshal.SizeOf(typeof(Startup));uint flags=4;
            if(helper){
                var security=new Security{size=(uint)Marshal.SizeOf(typeof(Security)),inherit=true};
                if(!CreatePipe(out inRead,out inWrite,ref security,0)||!CreatePipe(out outRead,out outWrite,ref security,0)||!CreatePipe(out errRead,out errWrite,ref security,0))throw new Win32Exception(Marshal.GetLastWin32Error());
                if(!SetHandleInformation(inWrite,1,0)||!SetHandleInformation(outRead,1,0)||!SetHandleInformation(errRead,1,0))throw new Win32Exception(Marshal.GetLastWin32Error());
                IntPtr size=IntPtr.Zero;InitializeProcThreadAttributeList(IntPtr.Zero,1,0,ref size);list=Marshal.AllocHGlobal(size);
                if(!InitializeProcThreadAttributeList(list,1,0,ref size))throw new Win32Exception(Marshal.GetLastWin32Error());listReady=true;
                handles=Marshal.AllocHGlobal(IntPtr.Size*3);Marshal.WriteIntPtr(handles,0,inRead);Marshal.WriteIntPtr(handles,IntPtr.Size,outWrite);Marshal.WriteIntPtr(handles,IntPtr.Size*2,errWrite);
                if(!UpdateProcThreadAttribute(list,0,new IntPtr(0x20002),handles,new IntPtr(IntPtr.Size*3),IntPtr.Zero,IntPtr.Zero))throw new Win32Exception(Marshal.GetLastWin32Error());
                startup.start.cb=(uint)Marshal.SizeOf(typeof(StartupEx));startup.start.flags=0x100;startup.start.input=inRead;startup.start.output=outWrite;startup.start.error=errWrite;startup.attributes=list;flags|=0x00080000|0x08000000;
            }
            var command=new StringBuilder(Quote(owner.image));foreach(string argument in arguments)command.Append(' ').Append(Quote(argument));
            ProcessInfo info;if(!CreateProcessW(owner.image,command,IntPtr.Zero,IntPtr.Zero,helper,flags,IntPtr.Zero,Path.GetDirectoryName(owner.image),ref startup,out info))throw new Win32Exception(Marshal.GetLastWin32Error());
            owner.processHandle=info.process;owner.threadHandle=info.thread;owner.pid=info.pid;owner.initialThread=info.tid;owner.SuspendedBeforeAssignment=true;
            if(!AssignProcessToJobObject(owner.job,owner.processHandle))throw new Win32Exception(Marshal.GetLastWin32Error());owner.assigned=true;
            FileTime c,e,k,u;if(!GetProcessTimes(owner.processHandle,out c,out e,out k,out u))throw new Win32Exception(Marshal.GetLastWin32Error());owner.created=c.Value;
            var loaded=new StringBuilder(32768);uint length=(uint)loaded.Capacity;
            if(!QueryFullProcessImageNameW(owner.processHandle,0,loaded,ref length)||!String.Equals(owner.image,loaded.ToString(),StringComparison.OrdinalIgnoreCase))throw new InvalidOperationException("Created image differs");
            owner.Process=System.Diagnostics.Process.GetProcessById(checked((int)owner.pid));
            if(!owner.IsOwnedJobMember(owner.created))throw new InvalidOperationException("Actual private Job membership required");
            owner.ExactImageVerifiedBeforeResume=true;
            if(helper){owner.stdout=owner.Drain(ref outRead,4*1024*1024);owner.stderr=owner.Drain(ref errRead,65536);}
            if(ResumeThread(owner.threadHandle)!=1)throw new Win32Exception(Marshal.GetLastWin32Error());owner.Resumed=true;
        } catch(Exception error){owner.StartupError=error.GetType().Name;} finally {
            foreach(IntPtr handle in new[]{inRead,inWrite,outRead,outWrite,errRead,errWrite})if(handle!=IntPtr.Zero)CloseHandle(handle);
            if(listReady)DeleteProcThreadAttributeList(list);if(list!=IntPtr.Zero)Marshal.FreeHGlobal(list);if(handles!=IntPtr.Zero)Marshal.FreeHGlobal(handles);
        }
        return owner;
    }
    public bool IsOwnedJobMember(ulong expectedCreated) {
        if(disposed||!assigned||expectedCreated==0||created!=expectedCreated||HasExited)return false;
        bool present;FileTime c,e,k,u;
        return IsProcessInJob(processHandle,job,out present)&&present&&GetProcessTimes(processHandle,out c,out e,out k,out u)&&c.Value==created;
    }
    public bool TrySelectWindow(string fixtureStem) {
        if(window!=IntPtr.Zero)return MatchesSelectedOwnedWindow(Window,windowThread,created);
        if(!IsOwnedJobMember(created))return false;
        IntPtr found=IntPtr.Zero;uint foundThread=0;int count=0;
        EnumWindow callback=(candidate,data)=>{uint candidatePid;uint tid=GetWindowThreadProcessId(candidate,out candidatePid);
            if(candidatePid==pid&&tid!=0&&IsWindowVisible(candidate)&&GetAncestor(candidate,2)==candidate&&GetWindow(candidate,4)==IntPtr.Zero){var title=new StringBuilder(1024);GetWindowTextW(candidate,title,title.Capacity);if(title.ToString().IndexOf(fixtureStem,StringComparison.Ordinal)>=0){found=candidate;foundThread=tid;count++;}}return true;};
        if(!EnumWindows(callback,IntPtr.Zero))throw new Win32Exception(Marshal.GetLastWin32Error());GC.KeepAlive(callback);
        if(count!=1)return false;window=found;windowThread=foundThread;return MatchesSelectedOwnedWindow(Window,windowThread,created);
    }
    public bool MatchesSelectedOwnedWindow(long expectedWindow,uint expectedThread,ulong expectedCreated) {
        if(window==IntPtr.Zero||expectedWindow!=Window||expectedThread!=windowThread||!IsOwnedJobMember(expectedCreated)||!IsWindow(window)||!IsWindowVisible(window)||GetAncestor(window,2)!=window||GetWindow(window,4)!=IntPtr.Zero)return false;
        uint actualPid;return GetWindowThreadProcessId(window,out actualPid)==windowThread&&actualPid==pid;
    }
    public bool IsForeground { get { return window!=IntPtr.Zero&&GetForegroundWindow()==window&&MatchesSelectedOwnedWindow(Window,windowThread,created); } }
    // Normal OS focus request only. No AttachThreadInput, synthetic keys, TOPMOST
    // or foreground-lock bypass; rejection is a bounded acceptance refusal.
    public void RequestNormalForeground() { if(!MatchesSelectedOwnedWindow(Window,windowThread,created))throw new InvalidOperationException("Owned selected root changed");ShowWindowAsync(window,9);SetForegroundWindow(window); }
    public void RequestRetirement() {
        if(processHandle==IntPtr.Zero)return;
        if(assigned){if(!TerminateJobObject(job,1))throw new Win32Exception(Marshal.GetLastWin32Error());}
        else if(!HasExited&&!TerminateProcess(processHandle,1))throw new Win32Exception(Marshal.GetLastWin32Error());
    }
    public void Dispose() {
        if(disposed)return;if(!IsSettled)throw new InvalidOperationException("Retained owner is Pending");disposed=true;
        if(Process!=null)Process.Dispose();if(threadHandle!=IntPtr.Zero)CloseHandle(threadHandle);if(processHandle!=IntPtr.Zero)CloseHandle(processHandle);if(job!=IntPtr.Zero)CloseHandle(job);
        threadHandle=processHandle=job=IntPtr.Zero;
    }
    public static void CreateBlankFixture(string path) {
        const int width=640,height=480,stride=width*3,size=stride*height;
        using(var file=new FileStream(path,FileMode.CreateNew,FileAccess.Write,FileShare.None))using(var writer=new BinaryWriter(file)){
            writer.Write((ushort)0x4d42);writer.Write(54+size);writer.Write(0);writer.Write(54);writer.Write(40);writer.Write(width);writer.Write(height);writer.Write((ushort)1);writer.Write((ushort)24);writer.Write(0);writer.Write(size);writer.Write(3780);writer.Write(3780);writer.Write(0);writer.Write(0);
            var row=new byte[stride];for(int i=0;i<row.Length;i++)row[i]=255;for(int y=0;y<height;y++)writer.Write(row);writer.Flush();file.Flush(true);
        }
    }
}

// Namespace leases deny rename/delete without changing any ACL. The installed
// Paint image deliberately uses only a final-file lease; Rust validates the OS
// package namespace under its separate installed-paint-v1 policy.
public static class M4FrameDirectory {
    [StructLayout(LayoutKind.Sequential)] private struct Info { public uint attributes; public System.Runtime.InteropServices.ComTypes.FILETIME created,accessed,written;public uint volume,sizeHigh,sizeLow,links,indexHigh,indexLow; }
    [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)] private static extern SafeFileHandle CreateFileW(string path,uint access,uint share,IntPtr security,uint mode,uint flags,IntPtr template);
    [DllImport("kernel32.dll",SetLastError=true)] private static extern bool GetFileInformationByHandle(SafeFileHandle handle,out Info info);
    [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)] private static extern uint GetFinalPathNameByHandleW(SafeFileHandle handle,StringBuilder path,uint length,uint flags);
    public static SafeFileHandle Open(string path) {
        string full=Path.GetFullPath(path);if(full.Length<3||full[1]!=':'||full[2]!='\\'||full.IndexOf(':',2)>=0)throw new IOException("Local directory required");
        var handle=CreateFileW(full,0x81,3,IntPtr.Zero,3,0x02200000,IntPtr.Zero);
        try{Info info;var text=new StringBuilder(32768);if(handle.IsInvalid||!GetFileInformationByHandle(handle,out info)||(info.attributes&0x10)==0||(info.attributes&0x400)!=0)throw new IOException("Directory lease refused");uint length=GetFinalPathNameByHandleW(handle,text,(uint)text.Capacity,0);if(length==0||length>=text.Capacity||!String.Equals(text.ToString(),"\\\\?\\"+full,StringComparison.OrdinalIgnoreCase))throw new IOException("Directory namespace changed");return handle;}catch{handle.Dispose();throw;}
    }
}
