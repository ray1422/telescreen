//! Start a helper process as its own responsible process (ADR 0005).
//!
//! macOS Local Network privacy blames a connection on the responsible process. A
//! child that the app starts with `responsibility_spawnattrs_setdisclaim` is its
//! own responsible process, and command-line tools are not subject to Local
//! Network privacy. The function is private API, so it is looked up at run time;
//! without it the child starts normally and the app keeps the responsibility.

use std::ffi::{CString, c_char, c_int, c_void};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disclaim {
    /// The child is its own responsible process.
    Disclaimed,
    /// `responsibility_spawnattrs_setdisclaim` is not available.
    NotAvailable,
}

unsafe extern "C" {
    static environ: *const *const c_char;
}

type SetDisclaim = unsafe extern "C" fn(*mut libc::posix_spawnattr_t, c_int) -> c_int;

/// Starts `program` with `args`. Returns the process ID and the disclaim state.
pub fn spawn_disclaimed(
    program: &Path,
    args: &[String],
) -> Result<(libc::pid_t, Disclaim), String> {
    let program_c =
        CString::new(program.as_os_str().as_encoded_bytes()).map_err(|e| e.to_string())?;
    let mut argv_c = vec![program_c.clone()];
    for arg in args {
        argv_c.push(CString::new(arg.as_bytes()).map_err(|e| e.to_string())?);
    }
    let mut argv: Vec<*mut c_char> = argv_c.iter().map(|a| a.as_ptr().cast_mut()).collect();
    argv.push(std::ptr::null_mut());

    // SAFETY: standard posix_spawn use. The attribute object is initialized,
    // used, and destroyed here. The strings live until posix_spawn returns.
    unsafe {
        let mut attr: libc::posix_spawnattr_t = std::ptr::null_mut();
        if libc::posix_spawnattr_init(&mut attr) != 0 {
            return Err("posix_spawnattr_init failed".into());
        }
        let symbol = libc::dlsym(
            libc::RTLD_DEFAULT,
            c"responsibility_spawnattrs_setdisclaim".as_ptr(),
        );
        let disclaim = if symbol.is_null() {
            Disclaim::NotAvailable
        } else {
            let set: SetDisclaim = std::mem::transmute::<*mut c_void, SetDisclaim>(symbol);
            if set(&mut attr, 1) == 0 {
                Disclaim::Disclaimed
            } else {
                Disclaim::NotAvailable
            }
        };
        let mut pid: libc::pid_t = 0;
        let error = libc::posix_spawn(
            &mut pid,
            program_c.as_ptr(),
            std::ptr::null(),
            &attr,
            argv.as_ptr(),
            environ.cast(),
        );
        libc::posix_spawnattr_destroy(&mut attr);
        if error != 0 {
            return Err(format!(
                "posix_spawn {}: {}",
                program.display(),
                std::io::Error::from_raw_os_error(error)
            ));
        }
        Ok((pid, disclaim))
    }
}
