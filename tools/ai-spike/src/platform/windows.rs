use crate::{
    Error, Result,
    request::{Prepared, ProviderResponse, parse_response},
};
use std::{
    ffi::c_void,
    ptr,
    sync::atomic::{Ordering, compiler_fence},
    time::Instant,
};
use windows::{
    Win32::{
        Networking::WinHttp::*,
        Security::Credentials::{CRED_TYPE_GENERIC, CREDENTIALW, CredFree, CredReadW},
    },
    core::{PCWSTR, w},
};

struct Secret(Vec<u16>);
impl Drop for Secret {
    fn drop(&mut self) {
        for value in &mut self.0 {
            // SAFETY: each pointer comes from an exclusive valid element of this owned buffer.
            unsafe {
                ptr::write_volatile(value, 0);
            }
        }
        compiler_fence(Ordering::SeqCst);
    }
}

fn authorization_headers(content_type: &str, key: &Secret) -> Result<Secret> {
    let prefix = format!("Content-Type: {content_type}\r\nAuthorization: Bearer ");
    let length = prefix
        .encode_utf16()
        .count()
        .checked_add(key.0.len())
        .and_then(|value| value.checked_add(2))
        .ok_or(Error::Limit("authorization header"))?;
    let mut headers = Secret(Vec::new());
    // Allocate the final capacity before copying secret material. Appending CRLF
    // afterward must never free an intermediate allocation containing the key.
    headers
        .0
        .try_reserve_exact(length)
        .map_err(|_| Error::Limit("authorization header allocation"))?;
    headers.0.extend(prefix.encode_utf16());
    headers.0.extend_from_slice(&key.0);
    headers.0.extend_from_slice(&[13, 10]);
    Ok(headers)
}

struct Credential(*mut CREDENTIALW);
impl Drop for Credential {
    fn drop(&mut self) {
        // SAFETY: successful CredReadW allocated this credential and its blob;
        // this owner frees it once after wiping only the documented blob extent.
        unsafe {
            let credential = &*self.0;
            if !credential.CredentialBlob.is_null() && credential.CredentialBlobSize <= 4096 {
                for index in 0..credential.CredentialBlobSize as usize {
                    ptr::write_volatile(credential.CredentialBlob.add(index), 0);
                }
            }
            compiler_fence(Ordering::SeqCst);
            CredFree(self.0.cast());
        }
    }
}

fn credential() -> Result<Secret> {
    let mut pointer = ptr::null_mut();
    // SAFETY: constant nul-terminated target, generic credential type, valid out pointer.
    unsafe {
        CredReadW(
            w!("VisualWorkbench/openai"),
            CRED_TYPE_GENERIC,
            None,
            &mut pointer,
        )
    }
    .map_err(|_| Error::Credential)?;
    if pointer.is_null() {
        return Err(Error::Credential);
    }
    let credential = Credential(pointer);
    // SAFETY: Credential owns a successful CredReadW allocation until this function ends.
    let value = unsafe { &*credential.0 };
    if value.CredentialBlob.is_null()
        || value.CredentialBlobSize == 0
        || value.CredentialBlobSize > 4096
        || !value.CredentialBlobSize.is_multiple_of(2)
        || value.UserName.is_null()
    {
        return Err(Error::Credential);
    }
    // SAFETY: the documented CREDENTIALW username is nul-terminated within the OS allocation.
    let username = unsafe { value.UserName.to_string() }.map_err(|_| Error::Credential)?;
    if username != "openai" {
        return Err(Error::Credential);
    }
    // SAFETY: the validated blob pointer and length remain alive through the Credential owner.
    let bytes = unsafe {
        std::slice::from_raw_parts(value.CredentialBlob, value.CredentialBlobSize as usize)
    };
    let mut key = Secret(
        bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect(),
    );
    if key.0.last() == Some(&0) {
        key.0.pop();
    }
    if key.0.is_empty()
        || key.0.len() > 2048
        || key.0.iter().any(|value| !(0x21..=0x7e).contains(value))
    {
        return Err(Error::Credential);
    }
    Ok(key)
}

struct Handle(*mut c_void);
impl Handle {
    fn new(pointer: *mut c_void) -> Result<Self> {
        if pointer.is_null() {
            Err(Error::Transport)
        } else {
            Ok(Self(pointer))
        }
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: this handle owns one successful WinHTTP allocation; all children
        // are declared later and drop first. No operation outlives this call.
        let _ = unsafe { WinHttpCloseHandle(self.0) };
    }
}

pub(super) fn send(prepared: &Prepared) -> Result<ProviderResponse> {
    let multipart = prepared.multipart()?;
    let key = credential()?;
    let started = Instant::now();
    let config = &prepared.description.provider;
    let timeout_ms = i32::try_from(config.timeout_seconds * 1000)
        .map_err(|_| Error::Limit("network timeout"))?;
    // SAFETY: all constant UTF-16 strings are terminated, handles are checked and
    // RAII-owned, buffers remain live for synchronous calls, and read lengths are
    // bounded by their allocations. No security/certificate bypass flags are set.
    unsafe {
        let session = Handle::new(WinHttpOpen(
            w!("VisualWorkbench-image-spike/0.1"),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        ))?;
        WinHttpSetTimeouts(
            session.0,
            10_000.min(timeout_ms),
            10_000.min(timeout_ms),
            timeout_ms,
            timeout_ms,
        )
        .map_err(|_| Error::Transport)?;
        let connection = Handle::new(WinHttpConnect(session.0, w!("api.openai.com"), 443, 0))?;
        let request = Handle::new(WinHttpOpenRequest(
            connection.0,
            w!("POST"),
            w!("/v1/images/edits"),
            PCWSTR::null(),
            PCWSTR::null(),
            ptr::null(),
            WINHTTP_FLAG_SECURE,
        ))?;
        WinHttpSetOption(
            Some(request.0.cast_const()),
            WINHTTP_OPTION_REDIRECT_POLICY,
            Some(&WINHTTP_OPTION_REDIRECT_POLICY_NEVER.to_ne_bytes()),
        )
        .map_err(|_| Error::Transport)?;
        let disabled = WINHTTP_DISABLE_COOKIES | WINHTTP_DISABLE_AUTHENTICATION;
        WinHttpSetOption(
            Some(request.0.cast_const()),
            WINHTTP_OPTION_DISABLE_FEATURE,
            Some(&disabled.to_ne_bytes()),
        )
        .map_err(|_| Error::Transport)?;
        let headers = authorization_headers(&multipart.content_type, &key)?;
        let body_len =
            u32::try_from(multipart.body.len()).map_err(|_| Error::Limit("request body"))?;
        WinHttpSendRequest(
            request.0,
            Some(&headers.0),
            Some(multipart.body.as_ptr().cast()),
            body_len,
            body_len,
            0,
        )
        .map_err(|_| Error::Transport)?;
        drop(headers);
        drop(key);
        if started.elapsed().as_secs() >= u64::from(config.timeout_seconds) {
            return Err(Error::Transport);
        }
        WinHttpReceiveResponse(request.0, ptr::null_mut()).map_err(|_| Error::Transport)?;
        let mut status = 0u32;
        let mut length = 4u32;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&mut status as *mut u32).cast()),
            &mut length,
            ptr::null_mut(),
        )
        .map_err(|_| Error::Transport)?;
        if status != 200 {
            return Err(Error::Http(status));
        }
        let mut response = Vec::new();
        let mut buffer = [0u8; 32 * 1024];
        loop {
            let remaining = u128::from(config.timeout_seconds) * 1000;
            let remaining = remaining
                .checked_sub(started.elapsed().as_millis())
                .filter(|value| *value > 0)
                .ok_or(Error::Transport)?;
            WinHttpSetTimeouts(
                request.0,
                10_000,
                10_000,
                timeout_ms,
                remaining.min(10_000) as i32,
            )
            .map_err(|_| Error::Transport)?;
            let mut read = 0u32;
            WinHttpReadData(
                request.0,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
                &mut read,
            )
            .map_err(|_| Error::Transport)?;
            if read == 0 {
                break;
            }
            let total = response
                .len()
                .checked_add(read as usize)
                .ok_or(Error::Limit("response bytes"))?;
            if total > config.max_response_bytes {
                return Err(Error::Limit("response bytes"));
            }
            response.extend_from_slice(&buffer[..read as usize]);
        }
        parse_response(
            &response,
            config.max_response_bytes,
            started.elapsed().as_millis(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{Secret, authorization_headers};
    #[test]
    fn complete_header_capacity_is_reserved_before_copying_long_secret() -> crate::Result<()> {
        let key = Secret(vec![u16::from(b'K'); 2048]);
        let content_type = "multipart/form-data; boundary=vw-synthetic-only";
        let headers = authorization_headers(content_type, &key)?;
        let prefix = format!("Content-Type: {content_type}\r\nAuthorization: Bearer ");
        let prefix_length = prefix.encode_utf16().count();
        assert_eq!(headers.0.len(), prefix_length + key.0.len() + 2);
        assert!(headers.0.capacity() >= headers.0.len());
        assert_eq!(
            &headers.0[prefix_length..prefix_length + key.0.len()],
            &key.0
        );
        assert_eq!(&headers.0[headers.0.len() - 2..], &[13, 10]);
        Ok(())
    }
}
