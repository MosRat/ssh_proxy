use std::{ffi::c_void, io, mem, ptr};

use anyhow::{Context, Result};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};

const SECURITY_DESCRIPTOR_REVISION: u32 = 1;
const CONTROL_PIPE_SDDL: &str = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;IU)";

pub(super) fn create_server(path: &str) -> Result<NamedPipeServer> {
    let security = PipeSecurityDescriptor::from_sddl(CONTROL_PIPE_SDDL)
        .context("failed to prepare named pipe security descriptor")?;
    let mut attributes = security.attributes();
    // Safety: attributes points to a valid SECURITY_ATTRIBUTES value whose
    // descriptor remains alive until CreateNamedPipeW returns.
    unsafe {
        ServerOptions::new()
            .create_with_security_attributes_raw(
                path,
                (&mut attributes as *mut SECURITY_ATTRIBUTES).cast::<c_void>(),
            )
            .with_context(|| format!("failed to create named pipe {path}"))
    }
}

struct PipeSecurityDescriptor {
    descriptor: PSECURITY_DESCRIPTOR,
}

impl PipeSecurityDescriptor {
    fn from_sddl(sddl: &str) -> Result<Self> {
        let wide = wide_null(sddl);
        let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
        // Safety: wide is null-terminated and descriptor points to writable
        // storage for the returned self-relative security descriptor pointer.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide.as_ptr(),
                SECURITY_DESCRIPTOR_REVISION,
                &mut descriptor,
                ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error())
                .with_context(|| format!("invalid named pipe SDDL {sddl:?}"));
        }
        Ok(Self { descriptor })
    }

    fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.descriptor.cast::<c_void>(),
            bInheritHandle: 0,
        }
    }
}

impl Drop for PipeSecurityDescriptor {
    fn drop(&mut self) {
        if !self.descriptor.is_null() {
            // Safety: ConvertStringSecurityDescriptorToSecurityDescriptorW
            // allocates this descriptor with LocalAlloc.
            unsafe {
                let _ = LocalFree(self.descriptor.cast::<c_void>());
            }
        }
    }
}

fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_pipe_security_descriptor_is_valid() {
        let descriptor = PipeSecurityDescriptor::from_sddl(CONTROL_PIPE_SDDL).unwrap();
        assert!(!descriptor.descriptor.is_null());
        let attributes = descriptor.attributes();
        assert_eq!(
            attributes.nLength,
            mem::size_of::<SECURITY_ATTRIBUTES>() as u32
        );
        assert!(!attributes.lpSecurityDescriptor.is_null());
    }
}
