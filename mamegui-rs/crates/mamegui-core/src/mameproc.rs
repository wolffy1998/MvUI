//! Wrapping the mame.exe child process (origin: processmanager.cpp + utils.cpp getMameVersion).

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, thiserror::Error)]
pub enum MameError {
    #[error("mame binary not found: {0}")]
    NotFound(String),
    #[error("failed to spawn mame: {0}")]
    Spawn(String),
    #[error("mame invocation failed: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone)]
pub struct MameBinary {
    pub path: PathBuf,
    pub version: String,
}

impl MameBinary {
    /// origin: utils->getMameVersion — runs `mame -help`, first output line
    pub fn detect(path: &Path) -> Result<Self, MameError> {
        if !path.exists() {
            return Err(MameError::NotFound(path.display().to_string()));
        }
        let mut cmd = quiet_command(path);
        cmd.arg("-help");
        let out = cmd.output();
        let version = match out {
            Ok(o) => String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty())
                .next()
                .unwrap_or("")
                .to_string(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self {
            path: path.to_path_buf(),
            version,
        })
    }

    pub fn spawn_listxml(&self) -> Result<Child, MameError> {
        self.run(&["-listxml"])
    }

    /// origin: second child — the option template source
    pub fn spawn_showconfig(&self) -> Result<Child, MameError> {
        self.run(&["-showconfig", "-noreadconfig"])
    }

    pub fn spawn_verifyroms(&self, game: Option<&str>) -> Result<Child, MameError> {
        match game {
            Some(g) => self.run(&["-verifyroms", g]),
            None => self.run(&["-verifyroms"]),
        }
    }

    pub fn spawn_verifysamples(&self, game: Option<&str>) -> Result<Child, MameError> {
        match game {
            Some(g) => self.run(&["-verifysamples", g]),
            None => self.run(&["-verifysamples"]),
        }
    }

    pub fn spawn_run(&self, args: &[String]) -> Result<Child, MameError> {
        self.run(args.iter().map(|s| s.as_str()).collect::<Vec<_>>().as_slice())
    }

    fn run(&self, args: &[&str]) -> Result<Child, MameError> {
        let mut cmd = quiet_command(&self.path);
        cmd.args(args);
        cmd.spawn()
            .map_err(|e| MameError::Spawn(format!("{} {:?}: {e}", self.path.display(), args)))
    }
}

/// A `Command` for mame.exe that never flashes a console window.
/// `detect()` used to build its command inline and therefore popped a console
/// on every start (origin: processmanager.cpp runs the checks detached).
fn quiet_command(path: &Path) -> Command {
    let mut cmd = Command::new(path);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

/// Read every line of the child stdout as it arrives, feeding a callback.
/// Blocks until EOF — call from a background thread.
pub fn pump_lines(child: &mut Child, mut on_line: impl FnMut(String)) -> Option<i32> {
    let stdout = child.stdout.take()?;
    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        match line {
            Ok(l) => on_line(l),
            Err(_) => break,
        }
    }
    match child.wait() {
        Ok(status) => status.code(),
        Err(_) => None,
    }
}
