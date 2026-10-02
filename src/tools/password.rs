use std::io::{self, IsTerminal, Write};

pub fn prompt(message: impl AsRef<str>) -> io::Result<String> {
    let mut output = io::stdout().lock();
    write!(output, "{}", message.as_ref())?;
    output.flush()?;
    if !io::stdin().is_terminal() {
        let mut value = String::new();
        io::stdin().read_line(&mut value)?;
        return Ok(value.trim_end_matches(['\r', '\n']).to_owned());
    }
    #[cfg(unix)]
    {
        let fd = libc::STDIN_FILENO;
        let mut original = std::mem::MaybeUninit::<libc::termios>::uninit();
        if unsafe { libc::tcgetattr(fd, original.as_mut_ptr()) } == 0 {
            let original = unsafe { original.assume_init() };
            let mut hidden = original;
            hidden.c_lflag &= !libc::ECHO;
            if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &hidden) } == 0 {
                let mut value = String::new();
                let result = io::stdin().read_line(&mut value);
                let _ = unsafe { libc::tcsetattr(fd, libc::TCSANOW, &original) };
                writeln!(output)?;
                return result.map(|_| value.trim_end_matches(['\r', '\n']).to_owned());
            }
        }
    }
    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    Ok(value.trim_end_matches(['\r', '\n']).to_owned())
}
