use axum::{Json, extract::State};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tokio::sync::{Mutex, Notify};

use crate::{ApiError, App, conflict, fail};

const ASSET: &str = "ampm-windows-x64.exe";
const MAX_BINARY: usize = 64 * 1024 * 1024;
pub const REPOSITORY: &str = match option_env!("AMPM_UPDATE_REPOSITORY") {
    Some(value) => value,
    None => "",
};

#[derive(Clone, Debug, Serialize)]
pub struct UpdateState {
    current: &'static str,
    repository: &'static str,
    supported: bool,
    auto_update: bool,
    status: &'static str,
    latest: Option<String>,
    checked_at: Option<u64>,
    error: Option<String>,
}

#[derive(Deserialize, Serialize)]
pub struct Preferences {
    auto_update: bool,
}

pub struct Updater {
    state: Mutex<UpdateState>,
    // Amp CLI and AMPM updates share a lock, including orderly shutdown.
    pub(crate) operation: Mutex<()>,
    preferences: PathBuf,
    client: reqwest::Client,
    pub visible: AtomicBool,
    pub editing: AtomicBool,
    pub closing: AtomicBool,
    pub restarting: AtomicBool,
    pub restart: Notify,
}

impl Updater {
    pub fn new(data: &Path, desktop: bool) -> Result<Self, Box<dyn std::error::Error>> {
        let preferences = data.join("updates.json");
        let auto_update = if preferences.exists() {
            serde_json::from_slice::<Preferences>(&fs::read(&preferences)?)?.auto_update
        } else {
            true
        };
        let supported =
            desktop && cfg!(all(windows, target_arch = "x86_64")) && valid_repository(REPOSITORY);
        Ok(Self {
            state: Mutex::new(UpdateState {
                current: env!("CARGO_PKG_VERSION"),
                repository: REPOSITORY,
                supported,
                auto_update,
                status: "idle",
                latest: None,
                checked_at: None,
                error: None,
            }),
            operation: Mutex::new(()),
            preferences,
            client: reqwest::Client::builder()
                .https_only(true)
                .user_agent(concat!("AMPM/", env!("CARGO_PKG_VERSION")))
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(120))
                .build()?,
            visible: AtomicBool::new(false),
            editing: AtomicBool::new(false),
            closing: AtomicBool::new(false),
            restarting: AtomicBool::new(false),
            restart: Notify::new(),
        })
    }

    pub async fn state(&self) -> UpdateState {
        self.state.lock().await.clone()
    }

    pub async fn finish(&self) {
        self.closing.store(true, Ordering::SeqCst);
        let _operation = self.operation.lock().await;
    }

    async fn report_error(&self, error: &str) {
        let mut state = self.state.lock().await;
        state.status = "error";
        state.error = Some(error.to_owned());
    }

    async fn ensure_installable(
        &self,
        manager: &mut crate::Manager,
        automatic: bool,
    ) -> Result<(), String> {
        if self.closing.load(Ordering::SeqCst) {
            return Err("AMPM 正在退出".into());
        }
        if self.editing.load(Ordering::SeqCst) {
            return Err("请先保存或关闭编辑窗口".into());
        }
        if automatic
            && (!self.state.lock().await.auto_update || self.visible.load(Ordering::SeqCst))
        {
            return Err("自动更新等待窗口收起".into());
        }
        if manager.processes.values_mut().any(|p| p.refresh()) {
            return Err("请先停止所有 Runner".into());
        }
        manager.ensure_writable().map_err(|e| e.1)
    }

    pub async fn check(&self) -> Result<UpdateState, ApiError> {
        let _operation = self
            .operation
            .try_lock()
            .map_err(|_| conflict("更新操作正在进行"))?;
        if self.closing.load(Ordering::SeqCst) || self.restarting.load(Ordering::SeqCst) {
            return Err(conflict("AMPM 正在退出或重启"));
        }
        if !self.state.lock().await.supported {
            return Err(fail("此构建未配置 Windows 桌面更新源"));
        }
        {
            let mut state = self.state.lock().await;
            state.status = "checking";
            state.error = None;
        }
        let result = async {
            let bytes = download(
                &self.client,
                &format!("https://api.github.com/repos/{REPOSITORY}/releases/latest"),
                1024 * 1024,
            )
            .await?;
            let release: Release =
                serde_json::from_slice(&bytes).map_err(|e| format!("无效的版本信息：{e}"))?;
            newer_release(&release, env!("CARGO_PKG_VERSION"))
        }
        .await;
        let mut state = self.state.lock().await;
        state.checked_at = Some(crate::process::now());
        match result {
            Ok(latest) => {
                state.latest = latest;
                state.status = if state.latest.is_some() {
                    "available"
                } else {
                    "current"
                };
                Ok(state.clone())
            }
            Err(error) => {
                state.status = "error";
                state.latest = None;
                state.error = Some(error.clone());
                Err(fail(error))
            }
        }
    }

    pub async fn install(&self, app: &App, automatic: bool) -> Result<UpdateState, ApiError> {
        let _operation = self
            .operation
            .try_lock()
            .map_err(|_| conflict("更新操作正在进行"))?;
        let state = self.state().await;
        if !state.supported {
            return Err(fail("此构建不支持自动安装更新"));
        }
        if self.restarting.load(Ordering::SeqCst) {
            return Err(conflict("正在重启"));
        }
        let latest = state.latest.ok_or_else(|| fail("请先检查更新"))?;
        self.ensure_installable(&mut *app.manager.lock().await, automatic)
            .await
            .map_err(conflict)?;
        {
            let mut state = self.state.lock().await;
            state.status = "downloading";
            state.error = None;
        }
        let result = async {
            let base = format!("https://github.com/{REPOSITORY}/releases/download/v{latest}");
            let checksum = download(&self.client, &format!("{base}/{ASSET}.sha256"), 1024).await?;
            let bytes = download(&self.client, &format!("{base}/{ASSET}"), MAX_BINARY).await?;
            verify_checksum(&bytes, &checksum)?;
            let mut staged = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
            staged.write_all(&bytes).map_err(|e| e.to_string())?;
            staged.as_file().sync_all().map_err(|e| e.to_string())?;
            // Hold the manager lock across the final decision and replacement. All
            // Runner starts/config writes must observe the maintenance flag afterward.
            let mut manager = app.manager.lock().await;
            self.restarting.store(true, Ordering::SeqCst);
            self.ensure_installable(&mut manager, automatic).await?;
            manager.updating = true;
            let replaced =
                tokio::task::spawn_blocking(move || replace_executable(staged.path())).await;
            if let Err(error) = replaced
                .map_err(|e| e.to_string())
                .and_then(|result| result)
            {
                manager.updating = false;
                return Err(error);
            }
            self.restarting.store(true, Ordering::SeqCst);
            self.restart.notify_one();
            Ok(())
        }
        .await;
        match result {
            Ok(()) => {
                self.state.lock().await.status = "restarting";
                Ok(self.state().await)
            }
            Err(error) => {
                self.restarting.store(false, Ordering::SeqCst);
                self.report_error(&error).await;
                Err(fail(error))
            }
        }
    }

    pub async fn background(app: App) {
        // No network activity in tests/headless mode. Only the desktop host starts this task.
        tokio::time::sleep(Duration::from_secs(30)).await;
        let mut last_check = None;
        let mut last_attempt = None;
        loop {
            let state = app.updater.state().await;
            if state.supported
                && state.auto_update
                && !app.updater.restarting.load(Ordering::SeqCst)
                && !app.updater.closing.load(Ordering::SeqCst)
            {
                if last_check.is_none_or(|t: std::time::Instant| {
                    t.elapsed() >= Duration::from_secs(6 * 3600)
                }) {
                    let _ = app.updater.check().await;
                    last_check = Some(std::time::Instant::now());
                }
                let state = app.updater.state().await;
                if state.latest.is_some()
                    && !app.updater.visible.load(Ordering::SeqCst)
                    && !app.updater.editing.load(Ordering::SeqCst)
                    && last_attempt.is_none_or(|t: std::time::Instant| {
                        t.elapsed() >= Duration::from_secs(3600)
                    })
                    && !app
                        .manager
                        .lock()
                        .await
                        .processes
                        .values_mut()
                        .any(|p| p.refresh())
                {
                    last_attempt = Some(std::time::Instant::now());
                    let _ = app.updater.install(&app, true).await;
                }
            }
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    }
}

fn valid_repository(repo: &str) -> bool {
    let parts: Vec<_> = repo.split('/').collect();
    parts.len() == 2
        && parts.iter().all(|p| {
            !p.is_empty()
                && *p != "."
                && *p != ".."
                && p.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
        })
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}
#[derive(Deserialize)]
struct Asset {
    name: String,
    size: u64,
}

fn newer_release(release: &Release, current: &str) -> Result<Option<String>, String> {
    let version = release
        .tag_name
        .strip_prefix('v')
        .ok_or("版本标签必须以 v 开头")?;
    let latest = Version::parse(version).map_err(|e| e.to_string())?;
    let current = Version::parse(current).map_err(|e| e.to_string())?;
    if release.draft
        || release.prerelease
        || !latest.pre.is_empty()
        || !latest.build.is_empty()
        || latest <= current
    {
        return Ok(None);
    }
    for (name, limit) in [
        (ASSET.to_owned(), MAX_BINARY),
        (format!("{ASSET}.sha256"), 1024),
    ] {
        let assets: Vec<_> = release
            .assets
            .iter()
            .filter(|asset| asset.name == name)
            .collect();
        if assets.len() != 1 || assets[0].size == 0 || assets[0].size > limit as u64 {
            return Err(format!("发布文件缺失或大小不正确：{name}"));
        }
    }
    Ok(Some(latest.to_string()))
}

async fn download(client: &reqwest::Client, url: &str, limit: usize) -> Result<Vec<u8>, String> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("连接 GitHub 失败：{e}"))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err("GitHub 尚未发布稳定版本或安装文件不存在".into());
    }
    let mut response = response
        .error_for_status()
        .map_err(|e| format!("GitHub 请求失败：{e}"))?;
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err("下载文件超过大小限制".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        if chunk.len() > limit.saturating_sub(bytes.len()) {
            return Err("下载文件超过大小限制".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn verify_checksum(binary: &[u8], checksum: &[u8]) -> Result<(), String> {
    let checksum = std::str::from_utf8(checksum).map_err(|_| "校验文件格式错误")?;
    let parts: Vec<_> = checksum.split_whitespace().collect();
    if parts.len() != 2
        || parts[1] != ASSET
        || parts[0].len() != 64
        || !parts[0].bytes().all(|c| c.is_ascii_hexdigit())
    {
        return Err("校验文件格式错误".into());
    }
    let actual = format!("{:x}", Sha256::digest(binary));
    if !actual.eq_ignore_ascii_case(parts[0]) {
        return Err("SHA-256 校验失败，已取消安装".into());
    }
    // Avoid replacing with a checksum-valid HTML/error page or a different architecture.
    if binary.len() < 64 || &binary[..2] != b"MZ" {
        return Err("安装文件不是 Windows 可执行文件".into());
    }
    let offset = u32::from_le_bytes(binary[60..64].try_into().unwrap()) as usize;
    if binary.get(offset..offset.saturating_add(6)) != Some(b"PE\0\0\x64\x86".as_slice()) {
        return Err("安装文件不是 Windows x64 程序".into());
    }
    Ok(())
}

#[cfg(windows)]
fn replace_executable(staged: &Path) -> Result<(), String> {
    let target = std::env::current_exe().map_err(|e| e.to_string())?;
    // self-replace moves the running exe away before installing the new file. Keep
    // our own backup to recover its documented post-rename failure window.
    let backup = tempfile::NamedTempFile::new_in(target.parent().ok_or("无法定位安装目录")?)
        .map_err(|e| format!("安装目录不可写：{e}"))?;
    fs::copy(&target, backup.path()).map_err(|e| e.to_string())?;
    if let Err(error) = self_replace::self_replace(staged) {
        if !target.exists()
            && let Err(restore) = backup.persist(&target)
        {
            let path = restore.file.path().to_path_buf();
            let _ = restore.file.keep();
            return Err(format!(
                "更新失败：{error}；恢复失败：{}；旧程序备份：{}",
                restore.error,
                path.display()
            ));
        }
        return Err(format!("更新失败，保留原版本：{error}"));
    }
    Ok(())
}
#[cfg(not(windows))]
fn replace_executable(_staged: &Path) -> Result<(), String> {
    Err("仅支持 Windows 桌面更新".into())
}

pub async fn check(State(app): State<App>) -> Result<Json<UpdateState>, ApiError> {
    app.updater.check().await.map(Json)
}
pub async fn install(State(app): State<App>) -> Result<Json<UpdateState>, ApiError> {
    app.updater.install(&app, false).await.map(Json)
}
pub async fn preferences(
    State(app): State<App>,
    Json(preferences): Json<Preferences>,
) -> Result<Json<UpdateState>, ApiError> {
    let mut state = app.updater.state.lock().await;
    let mut file = tempfile::NamedTempFile::new_in(app.updater.preferences.parent().unwrap())
        .map_err(|e| fail(e.to_string()))?;
    file.write_all(&serde_json::to_vec(&preferences).map_err(|e| fail(e.to_string()))?)
        .map_err(|e| fail(e.to_string()))?;
    file.as_file().sync_all().map_err(|e| fail(e.to_string()))?;
    file.persist(&app.updater.preferences)
        .map_err(|e| fail(e.to_string()))?;
    state.auto_update = preferences.auto_update;
    Ok(Json(state.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str) -> Release {
        Release {
            tag_name: tag.into(),
            draft: false,
            prerelease: false,
            assets: vec![
                Asset {
                    name: ASSET.into(),
                    size: 512,
                },
                Asset {
                    name: format!("{ASSET}.sha256"),
                    size: 90,
                },
            ],
        }
    }

    #[test]
    fn only_complete_newer_stable_releases_are_accepted() {
        assert_eq!(
            newer_release(&release("v0.10.0"), "0.9.9").unwrap(),
            Some("0.10.0".into())
        );
        for tag in ["v0.9.9", "v0.9.8", "v0.10.0-beta.1", "v0.10.0+build"] {
            assert!(newer_release(&release(tag), "0.9.9").unwrap().is_none());
        }
        assert!(newer_release(&release("../unsafe"), "0.9.9").is_err());
        for (draft, prerelease) in [(true, false), (false, true)] {
            let mut r = release("v1.0.0");
            r.draft = draft;
            r.prerelease = prerelease;
            assert!(newer_release(&r, "0.9.9").unwrap().is_none());
        }
        let mut r = release("v1.0.0");
        r.assets.pop();
        assert!(newer_release(&r, "0.9.9").is_err());
        let mut r = release("v1.0.0");
        r.assets[0].size = MAX_BINARY as u64 + 1;
        assert!(newer_release(&r, "0.9.9").is_err());
        assert!(valid_repository("meglinge/ampm"));
        for repo in ["", "../ampm", "owner/../repo", "user/repo?x=1"] {
            assert!(!valid_repository(repo));
        }
    }

    #[test]
    fn checksum_and_architecture_must_both_match() {
        let mut binary = vec![0u8; 128];
        binary[..2].copy_from_slice(b"MZ");
        binary[60..64].copy_from_slice(&64u32.to_le_bytes());
        binary[64..70].copy_from_slice(b"PE\0\0\x64\x86");
        let hash = |bytes: &[u8]| format!("{:x}  {ASSET}\n", Sha256::digest(bytes)).into_bytes();
        // Independently computed with .NET SHA256 for this fixed PE fixture.
        let checksum = b"d1a163b6d2e1ca2d24467cc99302770cca68a3bcc2e610f724fca69b410dd19a  ampm-windows-x64.exe\n";
        assert!(verify_checksum(&binary, checksum).is_ok());
        binary[100] = 1;
        assert!(
            verify_checksum(&binary, checksum)
                .unwrap_err()
                .contains("SHA-256")
        );
        binary[68] = 0x4c; // i386, not AMD64.
        assert!(
            verify_checksum(&binary, &hash(&binary))
                .unwrap_err()
                .contains("x64")
        );
        assert!(verify_checksum(b"<html>error</html>", &hash(b"<html>error</html>")).is_err());
        assert!(verify_checksum(&binary, b"not a checksum").is_err());
    }

    #[tokio::test]
    async fn preferences_persist_and_update_maintenance_blocks_runner_starts() {
        let directory = tempfile::tempdir().unwrap();
        let app = crate::tests::test_app(directory.path());
        let saved = preferences(State(app.clone()), Json(Preferences { auto_update: false }))
            .await
            .unwrap();
        assert!(!saved.0.auto_update);
        assert!(
            !Updater::new(directory.path(), false)
                .unwrap()
                .state()
                .await
                .auto_update
        );
        let mut manager = app.manager.lock().await;
        let mut config = manager.config.clone();
        config.runners.push(crate::tests::sample(directory.path()));
        manager.save(config).unwrap();
        manager.updating = true;
        assert_eq!(
            manager.action("test-runner", "start").await.unwrap_err().0,
            axum::http::StatusCode::CONFLICT
        );
        let config = manager.config.clone();
        assert!(manager.save(config).is_err());
    }

    #[tokio::test]
    async fn update_gates_and_operation_exclusion() {
        let directory = tempfile::tempdir().unwrap();
        let app = crate::tests::test_app(directory.path());
        let updater = &app.updater;
        let mut manager = app.manager.lock().await;
        assert!(updater.ensure_installable(&mut manager, true).await.is_ok());
        updater.visible.store(true, Ordering::SeqCst);
        assert!(
            updater
                .ensure_installable(&mut manager, true)
                .await
                .is_err()
        );
        assert!(
            updater
                .ensure_installable(&mut manager, false)
                .await
                .is_ok()
        );
        updater.visible.store(false, Ordering::SeqCst);
        updater.state.lock().await.auto_update = false;
        assert!(
            updater
                .ensure_installable(&mut manager, true)
                .await
                .is_err()
        );
        assert!(
            updater
                .ensure_installable(&mut manager, false)
                .await
                .is_ok()
        );
        for flag in [&updater.editing, &updater.closing] {
            flag.store(true, Ordering::SeqCst);
            assert!(
                updater
                    .ensure_installable(&mut manager, false)
                    .await
                    .is_err()
            );
            flag.store(false, Ordering::SeqCst);
        }
        let executable = directory
            .path()
            .join(if cfg!(windows) { "fake.exe" } else { "fake" });
        let compilation = std::process::Command::new("rustc")
            .args(["tests/fixtures/fake_amp.rs", "-o"])
            .arg(&executable)
            .output()
            .unwrap();
        assert!(
            compilation.status.success(),
            "{}",
            String::from_utf8_lossy(&compilation.stderr)
        );
        let mut runner = crate::tests::sample(directory.path());
        runner.executable = executable.to_string_lossy().into();
        let mut config = manager.config.clone();
        config.runners.push(runner);
        manager.save(config).unwrap();
        manager.action("test-runner", "start").await.unwrap();
        assert_eq!(
            updater
                .ensure_installable(&mut manager, false)
                .await
                .unwrap_err(),
            "请先停止所有 Runner"
        );
        manager.action("test-runner", "stop").await.unwrap();
        assert!(
            updater
                .ensure_installable(&mut manager, false)
                .await
                .is_ok()
        );
        drop(manager);
        let operation = updater.operation.lock().await;
        assert_eq!(
            updater.check().await.unwrap_err().0,
            axum::http::StatusCode::CONFLICT
        );
        assert_eq!(
            updater.install(&app, false).await.unwrap_err().0,
            axum::http::StatusCode::CONFLICT
        );
        drop(operation);
        assert!(!updater.restarting.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn downloads_reject_http_errors_truncation_and_unbounded_streams() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        for (response, accepted) in [
            ("HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\n1234", true),
            ("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n", false),
            ("HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n", false),
            ("HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\n12345", false),
            ("HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\n12", false),
            (
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\n123\r\n2\r\n45\r\n0\r\n\r\n",
                false,
            ),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut buffer = [0; 4096];
                assert!(stream.read(&mut buffer).await.unwrap() > 0);
                stream.write_all(response.as_bytes()).await.unwrap();
            });
            let result = download(&client, &format!("http://{address}/asset"), 4).await;
            assert_eq!(result.is_ok(), accepted, "{response}: {result:?}");
            if accepted {
                assert_eq!(result.unwrap(), b"1234");
            }
            server.await.unwrap();
        }
    }

    // Invoked only by the parent test on a COPY of the test executable.
    #[cfg(windows)]
    #[test]
    fn replacement_child() {
        let Some(stage) = std::env::var_os("AMPM_REPLACEMENT_TEST_STAGE") else {
            return;
        };
        let expected = std::env::var("AMPM_REPLACEMENT_TEST_SUCCESS").unwrap() == "true";
        assert_eq!(replace_executable(Path::new(&stage)).is_ok(), expected);
    }

    #[cfg(windows)]
    #[test]
    fn running_executable_is_replaced_and_failure_preserves_old_version() {
        let directory = tempfile::tempdir().unwrap();
        let current = std::env::current_exe().unwrap();
        let target = directory.path().join("installed.exe");
        let stage = directory.path().join("new.exe");
        let original = fs::read(current).unwrap();
        let mut updated = original.clone();
        updated.extend_from_slice(b"AMPM update test overlay");
        fs::write(&stage, &updated).unwrap();
        for success in [false, true] {
            fs::write(&target, &original).unwrap();
            let output = std::process::Command::new(&target)
                .args([
                    "--exact",
                    "updates::tests::replacement_child",
                    "--nocapture",
                ])
                .env(
                    "AMPM_REPLACEMENT_TEST_STAGE",
                    if success {
                        stage.clone()
                    } else {
                        directory.path().join("missing.exe")
                    },
                )
                .env("AMPM_REPLACEMENT_TEST_SUCCESS", success.to_string())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                fs::read(&target).unwrap(),
                if success { &updated } else { &original }.as_slice()
            );
            assert!(
                std::process::Command::new(&target)
                    .arg("--list")
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
        }
    }
}
