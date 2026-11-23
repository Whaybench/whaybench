//! `--token PROGRAM`: runs `PROGRAM setup-token` (Claude Code) in a terminal of this program's own, shows it as it is,
//! and takes the token it draws from the screen at the end, so the person need not copy and paste it.
use std::ffi::CString;

use crate::{size_of, write_all};

/// The token on the screen `claude setup-token` leaves: from `sk-ant-` to the first blank line, before the line under
/// it, with the line breaks the drawing put in taken out.
// shortcut: anchored on Claude Code's English line "Store this token"; if that wording changes, nothing is found and
// the person pastes the token instead.
pub fn found(screen: &str) -> Option<String> {
    let end = screen.find("Store this token")?;
    let start = screen[..end].rfind("sk-ant-")?;
    // A wrapped token's rows follow one another; a line after a blank one is something else.
    let t: String = screen[start..end].lines().take_while(|l| !l.trim().is_empty()).flat_map(str::split_whitespace).collect();
    wb_core::secret::claude_token_shaped(&t).then_some(t)
}

/// Runs the program and gives the token it showed, if it showed one.
pub fn take(program: &str) -> Result<Option<String>, String> {
    let size = size_of(0);
    let c_program = CString::new(program).map_err(|e| e.to_string())?;
    let c_arg = CString::new("setup-token").unwrap();
    let argv = [c_program.as_ptr(), c_arg.as_ptr(), std::ptr::null()];
    let mut saved: libc::termios = unsafe { std::mem::zeroed() };
    let have_saved = unsafe { libc::tcgetattr(0, &mut saved) } == 0;
    let mut master = -1;
    let pid = unsafe { libc::forkpty(&mut master, std::ptr::null_mut(), std::ptr::null_mut(), &size) };
    if pid < 0 { return Err(std::io::Error::last_os_error().to_string()); }
    if pid == 0 {
        unsafe {
            libc::execv(c_program.as_ptr(), argv.as_ptr());
            libc::_exit(127);
        }
    }
    crate::set_raw();
    let mut screen = vt100::Parser::new(size.ws_row, size.ws_col, 0);
    let mut buffer = vec![0u8; 65536];
    let mut input = 0;
    let mut reaped = false;
    loop {
        let mut fds = [
            libc::pollfd { fd: input, events: libc::POLLIN, revents: 0 },
            libc::pollfd { fd: master, events: libc::POLLIN, revents: 0 },
        ];
        match unsafe { libc::poll(fds.as_mut_ptr(), 2, 200) } {
            // Nothing to read and the program gone: something it started may hold the terminal open, so its end is
            // not waited for there.
            0 if unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) } == pid => { reaped = true; break; }
            n if n <= 0 => continue,
            _ => {}
        }
        if fds[1].revents != 0 {
            let got = unsafe { libc::read(master, buffer.as_mut_ptr().cast(), buffer.len()) };
            if got <= 0 { break; }
            screen.process(&buffer[..got as usize]);
            write_all(1, &buffer[..got as usize]);
        }
        if fds[0].revents != 0 {
            let got = unsafe { libc::read(0, buffer.as_mut_ptr().cast(), buffer.len()) };
            if got > 0 { write_all(master, &buffer[..got as usize]); } else { input = -1; }
        }
    }
    unsafe {
        if !reaped { libc::waitpid(pid, std::ptr::null_mut(), 0); }
        libc::close(master);
        // The shell after this reads a pasted token and Enter: the terminal goes back to how it was.
        if have_saved { libc::tcsetattr(0, libc::TCSANOW, &saved); }
    }
    Ok(found(&screen.screen().contents()))
}

#[cfg(test)]
mod tests {
    use super::found;

    #[test]
    fn token_read_off_the_screen() {
        let screen = "✓ Long-lived authentication token created\n\nYour OAuth token (valid for 1 year):\n\nsk-ant-oat01-abcdefghij\nKLMNOPQRST_uvwxyz-0123\n\nStore this token securely. You won't be able to see it again.\n";
        assert_eq!(found(screen).as_deref(), Some("sk-ant-oat01-abcdefghijKLMNOPQRST_uvwxyz-0123"));
        assert_eq!(found("sk-ant-oat01-abcdefghijKLMNOPQRST\nno line under it"), None);
        assert_eq!(found("sk-ant-oat01-abc def? ghijklmnopqrstuvwxyz\nStore this token"), None);
        // A line after the token is not part of it.
        assert_eq!(found("sk-ant-oat01-abcdefghijKLMNOPQRST\n\nCopied\nStore this token").as_deref(), Some("sk-ant-oat01-abcdefghijKLMNOPQRST"));
    }

    #[test]
    fn token_taken_from_a_program() {
        // Drawn as a TUI does: colours, a line break inside the token, the cursor moved over a line and back.
        let dir = std::env::temp_dir().join(format!("wb-token-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("claude");
        std::fs::write(&fake, "#!/bin/sh\nprintf 'Your OAuth token:\\r\\n\\033[33msk-ant-oat01-abcdefghij\\033[39m\\r\\n\\033[33mKLMNOPQRST_uv\\033[39m\\r\\nxx\\033[1A\\r\\033[2C\\r\\n\\033[2KStore this token securely.\\r\\n'\n").unwrap();
        std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let got = super::take(fake.to_str().unwrap());
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(got.unwrap().as_deref(), Some("sk-ant-oat01-abcdefghijKLMNOPQRST_uv"));
    }

    #[test]
    fn ends_when_the_program_does() {
        // What the program started (a browser, say) keeps the terminal open after it ends.
        let dir = std::env::temp_dir().join(format!("wb-token-hold-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("claude");
        std::fs::write(&fake, "#!/bin/sh\nsleep 30 &\nprintf 'sk-ant-oat01-abcdefghijKLMNOPQRST\\r\\n\\r\\nStore this token\\r\\n'\n").unwrap();
        std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let started = std::time::Instant::now();
        let got = super::take(fake.to_str().unwrap());
        let _ = std::fs::remove_dir_all(&dir);
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        assert_eq!(got.unwrap().as_deref(), Some("sk-ant-oat01-abcdefghijKLMNOPQRST"));
    }
}
