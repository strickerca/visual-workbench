//! Read-only physical-window candidates. Names are labels, never command proof.
use crate::{Error, Result};
use windows::Win32::{Foundation::*, System::Threading::*, UI::WindowsAndMessaging::*};
use windows::core::BOOL;
#[derive(Clone, Debug)]
pub struct WindowCandidate {
    pub window: u64,
    pub process_id: u32,
    pub process_created: u64,
    pub label: String,
    pub executable_name: String,
}
struct Census {
    owner: u32,
    seen: usize,
    values: Vec<WindowCandidate>,
}
/// SAFETY: EnumWindows passes the stack census only during this synchronous call.
unsafe extern "system" fn visit(hwnd: HWND, param: LPARAM) -> BOOL {
    let census = unsafe { &mut *(param.0 as *mut Census) };
    census.seen += 1;
    if census.seen > 1024 || census.values.len() >= 256 {
        return BOOL(0);
    }
    if !unsafe { IsWindowVisible(hwnd) }.as_bool() || unsafe { IsIconic(hwnd) }.as_bool() {
        return BOOL(1);
    }
    let window = hwnd.0 as usize as u64;
    let Ok(target) = crate::platform::query_target(
        window,
        census.owner,
        "018bcfe5-6800-7000-8000-000000000001".into(),
    ) else {
        return BOOL(1);
    };
    let mut label = [0u16; 512];
    let n = unsafe { GetWindowTextW(hwnd, &mut label) };
    if n <= 0 {
        return BOOL(1);
    }
    let executable_name = unsafe { image_name(target.process_id) }.unwrap_or_default();
    if crate::platform::unchanged(&target, census.owner).is_err() {
        return BOOL(1);
    }
    census.values.push(WindowCandidate {
        window,
        process_id: target.process_id,
        process_created: target.process_created,
        label: String::from_utf16_lossy(&label[..n as usize]),
        executable_name,
    });
    BOOL(1)
}
unsafe fn image_name(pid: u32) -> Result<String> {
    struct Process(HANDLE);
    impl Drop for Process {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
    let owner = Process(
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
            .map_err(|_| Error::Unavailable)?,
    );
    let mut path = vec![0u16; 32768];
    let mut count = path.len() as u32;
    unsafe {
        QueryFullProcessImageNameW(
            owner.0,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(path.as_mut_ptr()),
            &mut count,
        )
    }
    .map_err(|_| Error::Unavailable)?;
    Ok(String::from_utf16_lossy(&path[..count as usize])
        .rsplit('\\')
        .next()
        .unwrap_or_default()
        .into())
}
pub fn windows(owner: u32) -> Result<Vec<WindowCandidate>> {
    let mut census = Census {
        owner,
        seen: 0,
        values: Vec::new(),
    };
    // SAFETY: the census remains at this address throughout EnumWindows; no
    // pointer escapes. Stopping at bounded census is successful partial listing.
    let result = unsafe { EnumWindows(Some(visit), LPARAM((&mut census as *mut Census) as isize)) };
    if result.is_err() && census.seen <= 1024 && census.values.len() < 256 {
        return Err(Error::Unavailable);
    }
    Ok(census.values)
}

/// Metadata for an exact current selected target. A title/basename grants no capability.
pub fn describe(target: &vw_remote::Target, owner: u32) -> Result<String> {
    crate::platform::unchanged(target, owner)?;
    let hwnd = HWND(target.window as usize as *mut std::ffi::c_void);
    let mut text = [0u16; 512];
    // SAFETY: current exact selected HWND and initialized bounded output; metadata only.
    let n = unsafe { GetWindowTextW(hwnd, &mut text) };
    let title = if n > 0 {
        String::from_utf16_lossy(&text[..n as usize])
    } else {
        "Selected window".into()
    };
    // SAFETY: read-only retained target PID; revalidated again after the bounded query.
    let executable = unsafe { image_name(target.process_id) }.unwrap_or_default();
    crate::platform::unchanged(target, owner)?;
    let value = if executable.is_empty() {
        title
    } else {
        format!("{title} · {executable}")
    };
    let mut label = String::new();
    for c in value.chars().filter(|c| !c.is_control()) {
        if label.len() + c.len_utf8() > 2048 {
            break;
        }
        label.push(c);
    }
    Ok(label)
}
