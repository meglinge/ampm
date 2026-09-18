use serde::Serialize;
use std::{
    collections::VecDeque,
    process::Stdio,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::{Child, Command},
    time::{Duration, timeout},
};

use crate::{ApiError, Runner, fail};

pub type Logs = Arc<Mutex<VecDeque<LogLine>>>;

#[derive(Clone, Serialize)]
pub struct LogLine {
    pub time: u64,
    pub stream: String,
    pub text: String,
}

pub fn log(logs: &Logs, stream: &str, text: impl Into<String>) {
    let mut lines = logs.lock().unwrap_or_else(|e| e.into_inner());
    lines.push_back(LogLine {
        time: now(),
        stream: stream.into(),
        text: text.into(),
    });
    while lines.len() > 300 {
        lines.pop_front();
    }
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn capture(
    reader: impl AsyncRead + Unpin + Send + 'static,
    logs: Logs,
    stream: &'static str,
    wsl_pid: Arc<Mutex<Option<u32>>>,
    read_pid: bool,
) {
    tokio::spawn(async move {
        let mut reader = reader;
        let mut buffer = [0; 2048];
        let mut pending = Vec::new();
        let mut first_line = read_pid;
        loop {
            let count = match reader.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            for byte in &buffer[..count] {
                if *byte == b'\n' || pending.len() >= 8192 {
                    let text = String::from_utf8_lossy(&pending)
                        .trim_end_matches('\r')
                        .to_string();
                    if let Some(pid) = first_line
                        .then(|| {
                            text.strip_prefix("AMPM_WSL_PID=")
                                .and_then(|v| v.parse::<u32>().ok())
                                .filter(|v| *v > 1)
                        })
                        .flatten()
                    {
                        *wsl_pid.lock().unwrap_or_else(|e| e.into_inner()) = Some(pid);
                    } else if !text.is_empty() {
                        log(&logs, stream, text);
                    }
                    first_line = false;
                    pending.clear();
                    if *byte == b'\n' {
                        continue;
                    }
                }
                pending.push(*byte);
            }
        }
        if !pending.is_empty() {
            log(&logs, stream, String::from_utf8_lossy(&pending).to_string());
        }
    });
}

pub fn startup_args(runner: &Runner) -> Vec<String> {
    let mut args = vec![
        "--no-tui".into(),
        "--runner-id".into(),
        runner.id.clone(),
        "--no-color".into(),
    ];
    args.push(
        if runner.terminal {
            "--remote-control-terminal"
        } else {
            "--no-remote-control-terminal"
        }
        .into(),
    );
    if runner.discover {
        args.push("--discover-dirs".into());
    }
    // Amp always serves its process cwd. Keep this CLI detail out of the folder model.
    for path in runner.directories.iter().skip(1) {
        args.extend(["--dir".into(), path.clone()]);
    }
    args
}

pub fn native_command(executable: &str) -> Command {
    #[cfg(windows)]
    {
        let amp_home = std::env::var_os("AMP_HOME")
            .filter(|value| !value.is_empty())
            .map(std::path::PathBuf::from)
            .or_else(|| directories::BaseDirs::new().map(|dirs| dirs.home_dir().join(".amp")));
        Command::new(resolve_amp(
            executable,
            std::env::var_os("PATH").as_deref(),
            amp_home.as_deref(),
        ))
    }
    #[cfg(not(windows))]
    Command::new(executable)
}

#[cfg(windows)]
pub(crate) fn resolve_amp(
    executable: &str,
    path: Option<&std::ffi::OsStr>,
    amp_home: Option<&std::path::Path>,
) -> std::path::PathBuf {
    // Explorer may still have the PATH from before Amp was installed. Only the default
    // command gets an installer-location fallback; explicit paths and custom CLIs stay intact.
    if executable.eq_ignore_ascii_case("amp") || executable.eq_ignore_ascii_case("amp.exe") {
        let from_path = path.into_iter().flat_map(std::env::split_paths);
        let install_bin = amp_home.map(|home| home.join("bin"));
        if let Some(found) = from_path
            .chain(install_bin)
            .filter(|directory| directory.is_absolute())
            .map(|directory| directory.join("amp.exe"))
            .find(|candidate| candidate.is_file())
        {
            return found;
        }
    }
    executable.into()
}

fn spawn_error(cmd: &Command, error: std::io::Error) -> ApiError {
    let executable = cmd.as_std().get_program().to_string_lossy();
    if error.kind() == std::io::ErrorKind::NotFound {
        fail(format!(
            "找不到可执行文件：{executable}。请在“编辑”中填写完整路径，并检查运行环境。"
        ))
    } else {
        fail(format!("无法执行 {executable}：{error}"))
    }
}

pub fn command(runner: &Runner, args: &[String], supervised: bool) -> Command {
    // Validated runners always have a directory. Preserve list order across restarts
    // because Amp's dynamic directory memory and discovery are scoped to this cwd.
    let cwd = &runner.directories[0];
    let mut cmd = if runner.backend == "wsl" {
        let mut cmd = Command::new("wsl.exe");
        if !runner.distribution.is_empty() {
            cmd.args(["--distribution", &runner.distribution]);
        }
        cmd.args(["--cd", cwd, "--exec"]);
        if supervised {
            // A fixed script, positional arguments only. Never interpolate user paths into shell code.
            cmd.args([
                "setsid",
                "--wait",
                "sh",
                "-c",
                "printf 'AMPM_WSL_PID=%s\\n' \"$$\"; exec \"$@\"",
                "ampm",
            ]);
        }
        cmd.arg(&runner.executable).args(args);
        cmd
    } else {
        let mut cmd = native_command(&runner.executable);
        cmd.current_dir(cwd).args(args);
        cmd
    };
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    #[cfg(unix)]
    if supervised {
        cmd.process_group(0);
    }
    cmd
}

pub async fn output(mut cmd: Command) -> Result<String, ApiError> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);
    let child = cmd.spawn().map_err(|e| spawn_error(&cmd, e))?;
    let result = timeout(Duration::from_secs(15), child.wait_with_output())
        .await
        .map_err(|_| fail("Amp 命令超时（15 秒），请检查 CLI 登录与运行状态"))?
        .map_err(|e| fail(e.to_string()))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    if !result.status.success() {
        return Err(fail(format!("Amp 命令失败：{}", text.trim())));
    }
    Ok(text)
}

pub struct Running {
    pub child: Child,
    pub logs: Logs,
    pub started: u64,
    pub exit: Option<String>,
    pub initial_directories: Vec<String>,
    wsl_pid: Arc<Mutex<Option<u32>>>,
    #[cfg(windows)]
    job: Option<Job>,
    #[cfg(unix)]
    group: Option<u32>,
}

impl Running {
    pub async fn spawn(runner: &Runner) -> Result<Self, ApiError> {
        let mut cmd = command(runner, &startup_args(runner), true);
        let mut child = cmd.spawn().map_err(|e| spawn_error(&cmd, e))?;
        #[cfg(windows)]
        let job = if runner.backend == "native" {
            match Job::attach(&child) {
                Ok(job) => Some(job),
                Err(e) => {
                    let _ = child.kill().await;
                    return Err(fail(format!("无法托管进程树：{e}")));
                }
            }
        } else {
            None
        };
        let logs = Logs::default();
        let wsl_pid = Arc::new(Mutex::new(None));
        log(
            &logs,
            "manager",
            format!(
                "启动 {} · PID {} · 进程运行不代表已连接 Amp",
                runner.id,
                child.id().unwrap_or_default()
            ),
        );
        if let Some(stdout) = child.stdout.take() {
            capture(
                stdout,
                logs.clone(),
                "stdout",
                wsl_pid.clone(),
                runner.backend == "wsl",
            );
        }
        if let Some(stderr) = child.stderr.take() {
            capture(stderr, logs.clone(), "stderr", wsl_pid.clone(), false);
        }
        Ok(Self {
            #[cfg(unix)]
            group: child.id(),
            child,
            logs,
            wsl_pid,
            started: now(),
            exit: None,
            initial_directories: runner.directories.clone(),
            #[cfg(windows)]
            job,
        })
    }

    pub fn refresh(&mut self) -> bool {
        if self.exit.is_some() {
            return false;
        }
        match self.child.try_wait() {
            Ok(None) => true,
            Ok(Some(status)) => {
                let message =
                    format!("进程已退出：{status}。若由 Amp 更新触发，请检查后重新启动。");
                log(&self.logs, "manager", &message);
                self.exit = Some(message);
                #[cfg(windows)]
                {
                    self.job.take();
                }
                #[cfg(unix)]
                self.kill_group(libc::SIGKILL);
                false
            }
            Err(e) => {
                log(&self.logs, "stderr", e.to_string());
                true
            }
        }
    }

    #[cfg(unix)]
    fn kill_group(&mut self, signal: i32) {
        if let Some(pid) = self.group {
            unsafe {
                libc::kill(-(pid as i32), signal);
            }
            if signal == libc::SIGKILL {
                self.group = None;
            }
        }
    }

    pub async fn stop(&mut self, runner: &Runner) -> Result<(), ApiError> {
        if !self.refresh() {
            return Ok(());
        }
        if runner.backend == "wsl" {
            let pid = *self.wsl_pid.lock().unwrap_or_else(|e| e.into_inner());
            let pid = pid.ok_or_else(|| {
                fail("WSL 尚未返回进程组 ID，请稍后重试停止；不会终止整个 WSL 发行版")
            })?;
            let mut cmd = Command::new("wsl.exe");
            if !runner.distribution.is_empty() {
                cmd.args(["--distribution", &runner.distribution]);
            }
            cmd.args(["--exec", "kill", "-KILL", "--", &format!("-{pid}")])
                .kill_on_drop(true);
            output(cmd).await?;
        }
        #[cfg(windows)]
        {
            self.job.take();
        }
        #[cfg(unix)]
        self.kill_group(libc::SIGTERM);
        #[cfg(windows)]
        let _ = self.child.start_kill();
        match timeout(Duration::from_secs(3), self.child.wait()).await {
            Ok(result) => {
                result.map_err(|e| fail(e.to_string()))?;
            }
            Err(_) => {
                #[cfg(unix)]
                self.kill_group(libc::SIGKILL);
                self.child.kill().await.map_err(|e| fail(e.to_string()))?;
            }
        }
        #[cfg(unix)]
        self.kill_group(libc::SIGKILL);
        self.exit = Some("由管理器停止".into());
        log(&self.logs, "manager", "Runner 已停止");
        Ok(())
    }
}

#[cfg(unix)]
impl Drop for Running {
    fn drop(&mut self) {
        self.kill_group(libc::SIGKILL);
    }
}

#[cfg(windows)]
struct Job(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
unsafe impl Send for Job {}

#[cfg(windows)]
impl Job {
    fn attach(child: &Child) -> std::io::Result<Self> {
        use windows_sys::Win32::System::JobObjects::*;
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            let job = Self(handle);
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &info as *const _ as _,
                std::mem::size_of_val(&info) as u32,
            ) == 0
                || AssignProcessToJobObject(
                    handle,
                    child
                        .raw_handle()
                        .ok_or_else(|| std::io::Error::other("进程句柄不可用"))?
                        as _,
                ) == 0
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(job)
        }
    }
}

#[cfg(windows)]
impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
