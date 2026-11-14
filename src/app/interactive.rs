use super::*;

pub(super) fn require_terminal(json: bool) -> Result<()> {
    if json || !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        bail!(
            "init requires a human interactive TTY and does not accept --json; Agents must use setup --json, obtain user confirmation, then apply <plan-id> --json"
        );
    }
    Ok(())
}

pub(super) fn init_command(
    store: &StateStore,
    home: &Path,
    state_dir: &Path,
    roots: &[String],
    source_roots: &[PathBuf],
    args: &crate::cli::InitArgs,
    state_lock: &change::StateLock,
) -> Result<(Value, Vec<SuggestedAction>)> {
    inspection::ensure_snapshot(store, home, state_dir, roots, source_roots)
        .map_err(error::before_mutation)?;
    let mut setup = setup_command_for_agent(
        store,
        home,
        state_dir,
        args.modified_choice,
        false,
        args.agent,
    )
    .map_err(error::before_mutation)?;
    let Some(id) = setup["plan_id"].as_str() else {
        return Ok((setup, vec![]));
    };
    eprintln!("{}\n", crate::present::human("setup", &setup));
    eprintln!("{}\n", human_plan_preview(store, id)?);
    // Exact stored operations are shown before the single human confirmation.
    let detail = plan_detail_command(store, id).map_err(error::before_mutation)?;
    for operation in detail["operations"].as_array().into_iter().flatten() {
        eprintln!(
            "  {}: {}",
            operation["kind"].as_str().unwrap_or("operation"),
            operation["target"].as_str().unwrap_or_default()
        );
        if let Some(source) = operation["source"].as_str() {
            eprintln!("    from: {source}");
        }
    }
    if !confirm(&format!("Apply Bootstrap Plan {id}?")).map_err(error::before_mutation)? {
        setup["status"] = json!("cancelled");
        return Ok((setup, vec![]));
    }
    let mut result = apply_command(store, id, state_lock)?;
    let receipt = result["receipt_id"].as_str().unwrap_or_default().to_owned();
    let rescan_required = result["verification"] == "passed" && result["status"] == "applied";
    result["rescan_required"] = json!(rescan_required);
    result["agent_prompt"] = json!(
        "Use SkillRoster to check my local Skills and suggest a plan; ask me before applying any changes."
    );
    let mut actions = Vec::new();
    if rescan_required {
        actions.push(snapshot_rescan_action());
    }
    if result["undo_available"] == true {
        actions.push(receipt_undo_action(&receipt));
    }
    Ok((result, actions))
}

fn confirm(question: &str) -> Result<bool> {
    let _mode = TerminalMode::enter()?;
    eprintln!("{question}\nEnter confirm · Esc cancel");
    std::io::stderr().flush()?;
    let mut key = [0];
    let read = std::io::stdin().read(&mut key)?;
    if read == 1 && key[0] == 27 {
        // Allow a terminal's multi-byte Escape key sequence to finish arriving
        // before TerminalMode discards pending input and restores the shell mode.
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    Ok(read == 1 && matches!(key[0], b'\r' | b'\n'))
}

#[cfg(unix)]
struct TerminalMode(libc::termios);

#[cfg(unix)]
impl TerminalMode {
    fn enter() -> Result<Self> {
        let mut previous = std::mem::MaybeUninit::uninit();
        // SAFETY: stdin is a TTY; termios is initialized by tcgetattr on success.
        if unsafe { libc::tcgetattr(libc::STDIN_FILENO, previous.as_mut_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let previous = unsafe { previous.assume_init() };
        let mut mode = previous;
        // Read Ctrl-C as cancellation so this guard restores the mode too.
        mode.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG);
        mode.c_cc[libc::VMIN] = 1;
        mode.c_cc[libc::VTIME] = 0;
        if unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &mode) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(Self(previous))
    }
}

#[cfg(unix)]
impl Drop for TerminalMode {
    fn drop(&mut self) {
        unsafe {
            // Discard the remainder of Escape/arrow/pasted key sequences before
            // handing canonical input back to the shell.
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSAFLUSH, &self.0);
        }
    }
}

#[cfg(windows)]
struct TerminalMode(windows_sys::Win32::Foundation::HANDLE, u32);

#[cfg(windows)]
impl TerminalMode {
    fn enter() -> Result<Self> {
        use windows_sys::Win32::System::Console::*;
        let handle = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
        let mut previous = 0;
        if unsafe { GetConsoleMode(handle, &mut previous) } == 0
            || unsafe {
                SetConsoleMode(
                    handle,
                    previous & !(ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT | ENABLE_PROCESSED_INPUT),
                )
            } == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(Self(handle, previous))
    }
}

#[cfg(windows)]
impl Drop for TerminalMode {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::System::Console::FlushConsoleInputBuffer(self.0);
            windows_sys::Win32::System::Console::SetConsoleMode(self.0, self.1);
        }
    }
}
