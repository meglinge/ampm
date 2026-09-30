use axum::{
    Json,
    extract::{Query, State},
};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::atomic::Ordering, time::Duration};
use tokio::{process::Command, sync::Mutex};

use crate::{ApiError, App, Runner, conflict, fail, process};

// The same stable-version endpoint used by https://ampcode.com/install.ps1.
const VERSION_URL: &str = "https://static.ampcode.com/cli/cli-version.txt";

#[derive(Clone, Debug, Default, Serialize)]
pub struct CliState {
    available: bool,
    version: Option<String>,
    executable: String,
    latest: Option<String>,
    status: &'static str,
    checked_at: Option<u64>,
    error: Option<String>,
    #[serde(skip)]
    runner: Option<Runner>,
}

pub struct Cli {
    pub states: Mutex<HashMap<String, CliState>>,
    client: reqwest::Client,
}

impl Cli {
    pub fn new() -> Result<Self, reqwest::Error> {
        Ok(Self {
            states: Mutex::new(HashMap::new()),
            client: reqwest::Client::builder()
                .https_only(true)
                .user_agent(concat!("AMPM/", env!("CARGO_PKG_VERSION")))
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(20))
                .build()?,
        })
    }

    async fn inspect(
        &self,
        app: &App,
        id: &str,
        source: Option<&str>,
    ) -> Result<CliState, ApiError> {
        let _operation = app
            .updater
            .operation
            .try_lock()
            .map_err(|_| conflict("更新或检测正在进行，请稍后重试"))?;
        ensure_open(app)?;
        let runner = target(app, id).await?;
        let mut state = CliState {
            executable: executable(runner.as_ref()),
            runner,
            status: "checking",
            ..Default::default()
        };
        self.states.lock().await.insert(id.into(), state.clone());
        let result = async {
            state.version = Some(installed(state.runner.as_ref()).await?);
            state.available = true;
            state.status = "idle";
            if let Some(url) = source {
                let version = latest_version(&self.client, url).await?;
                state.latest = Some(version.clone());
                state.status = compare(state.version.as_deref().unwrap(), &version)?;
                state.checked_at = Some(process::now());
            }
            Ok(())
        }
        .await;
        self.complete(id, state, result).await
    }

    async fn install(&self, app: &App, id: &str) -> Result<CliState, ApiError> {
        let _operation = app
            .updater
            .operation
            .try_lock()
            .map_err(|_| conflict("更新或检测正在进行，请稍后重试"))?;
        ensure_open(app)?;
        let mut state = self
            .states
            .lock()
            .await
            .get(id)
            .cloned()
            .filter(|s| s.status == "available")
            .ok_or_else(|| fail("请先检查 Amp 更新"))?;
        let latest = state
            .latest
            .clone()
            .ok_or_else(|| fail("请先检查 Amp 更新"))?;
        {
            let mut manager = app.manager.lock().await;
            manager.ensure_writable()?;
            let runner = if id.is_empty() {
                None
            } else {
                Some(manager.runner(id)?)
            };
            if runner != state.runner || executable(runner.as_ref()) != state.executable {
                return Err(conflict("Amp 路径或 Runner 配置已改变，请重新检查更新"));
            }
            if manager.processes.values_mut().any(|p| p.refresh()) {
                return Err(conflict("请先停止所有托管 Runner，再更新 Amp"));
            }
            manager.cli_updating = true;
        }
        state.status = "updating";
        state.error = None;
        self.states.lock().await.insert(id.into(), state.clone());
        // Do not hold the manager mutex during a download. The maintenance flag blocks
        // starts/config writes, while state polling and the process monitor stay responsive.
        let result = async {
            state.version = Some(installed(state.runner.as_ref()).await?);
            if compare(state.version.as_deref().unwrap(), &latest)? == "available" {
                process::output_with_timeout(
                    command(
                        state.runner.as_ref(),
                        &["update".into(), "--porcelain".into()],
                    ),
                    Duration::from_secs(300),
                )
                .await?;
                // An exit code alone is insufficient: old wrappers may report success
                // without replacing the selected installation.
                state.available = false;
                state.version = None;
                state.version = Some(installed(state.runner.as_ref()).await?);
                state.available = true;
            }
            state.status = compare(state.version.as_deref().unwrap(), &latest)?;
            if state.status == "available" {
                return Err(fail(
                    "更新命令已结束，但所选 Amp 仍是旧版本；请检查安装路径或包管理器后重试",
                ));
            }
            Ok(())
        }
        .await;
        app.manager.lock().await.cli_updating = false;
        self.complete(id, state, result).await
    }

    async fn complete(
        &self,
        id: &str,
        mut state: CliState,
        result: Result<(), ApiError>,
    ) -> Result<CliState, ApiError> {
        if let Err(error) = &result {
            state.status = "error";
            state.error = Some(error.1.clone());
        }
        self.states.lock().await.insert(id.into(), state.clone());
        result.map(|()| state)
    }
}

fn ensure_open(app: &App) -> Result<(), ApiError> {
    if app.updater.closing.load(Ordering::SeqCst) || app.updater.restarting.load(Ordering::SeqCst) {
        return Err(conflict("AMPM 正在退出或重启"));
    }
    Ok(())
}

async fn target(app: &App, id: &str) -> Result<Option<Runner>, ApiError> {
    if id.is_empty() {
        Ok(None)
    } else {
        app.manager.lock().await.runner(id).map(Some)
    }
}

fn command(runner: Option<&Runner>, args: &[String]) -> Command {
    match runner {
        Some(runner) => process::command(runner, args, false),
        None => {
            let mut cmd = process::native_command("amp");
            cmd.args(args);
            cmd
        }
    }
}

fn executable(runner: Option<&Runner>) -> String {
    if let Some(runner) = runner.filter(|r| r.backend == "wsl") {
        runner.executable.clone()
    } else {
        command(runner, &[])
            .as_std()
            .get_program()
            .to_string_lossy()
            .into_owned()
    }
}

async fn installed(runner: Option<&Runner>) -> Result<String, ApiError> {
    process::output(command(runner, &["--version".into()]))
        .await
        .map(|s| s.trim().to_owned())
}

fn version(text: &str) -> Result<Version, ApiError> {
    Version::parse(text.split_whitespace().next().unwrap_or(""))
        .map_err(|_| fail("无法识别 Amp 版本号，请检查所选 CLI"))
}

fn compare(current: &str, latest: &str) -> Result<&'static str, ApiError> {
    let current = version(current)?;
    let latest = version(latest)?;
    // Amp's patch number is a build timestamp; the g<commit> suffix is not an
    // ordered prerelease. Never downgrade or sort git hashes lexicographically.
    Ok(
        match (current.major, current.minor, current.patch).cmp(&(
            latest.major,
            latest.minor,
            latest.patch,
        )) {
            std::cmp::Ordering::Less => "available",
            std::cmp::Ordering::Equal => "current",
            std::cmp::Ordering::Greater => "newer",
        },
    )
}

async fn latest_version(client: &reqwest::Client, url: &str) -> Result<String, ApiError> {
    let mut response = client
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| fail(format!("无法获取 Amp 最新版本：{e}")))?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| fail(format!("读取 Amp 版本失败：{e}")))?
    {
        if bytes.len() + chunk.len() > 256 {
            return Err(fail("Amp 版本响应过长"));
        }
        bytes.extend_from_slice(&chunk);
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| fail("Amp 版本响应不是 UTF-8"))?
        .trim();
    // Unlike --version output, the metadata must contain only a single version.
    Version::parse(text).map_err(|_| fail("Amp 更新源返回了无效版本"))?;
    Ok(text.into())
}

#[derive(Default, Deserialize)]
pub struct Selection {
    #[serde(default)]
    runner_id: String,
}

pub async fn info(
    State(app): State<App>,
    Query(selection): Query<Selection>,
) -> Result<Json<CliState>, ApiError> {
    tokio::spawn(async move { app.cli.inspect(&app, &selection.runner_id, None).await })
        .await
        .map_err(|e| fail(e.to_string()))?
        .map(Json)
}

pub async fn check(
    State(app): State<App>,
    Json(selection): Json<Selection>,
) -> Result<Json<CliState>, ApiError> {
    tokio::spawn(async move {
        app.cli
            .inspect(&app, &selection.runner_id, Some(VERSION_URL))
            .await
    })
    .await
    .map_err(|e| fail(e.to_string()))?
    .map(Json)
}

pub async fn update(
    State(app): State<App>,
    Json(selection): Json<Selection>,
) -> Result<Json<CliState>, ApiError> {
    // A disconnected GUI must not cancel an in-progress installation or strand its
    // maintenance flag. Shutdown waits for the shared operation lock in serve().
    tokio::spawn(async move { app.cli.install(&app, &selection.runner_id).await })
        .await
        .map_err(|e| fail(e.to_string()))?
        .map(Json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{request, sample, test_app};
    use axum::{Router, http::StatusCode, routing::get};
    use serde_json::json;
    use std::{fs, sync::Arc};

    #[test]
    fn compares_build_numbers_not_git_hashes() {
        for (current, latest, expected) in [
            (
                "0.0.99-gffffff (released yesterday)",
                "0.0.100-g000001",
                "available",
            ),
            ("0.0.101-g000001", "0.0.100-gffffff", "newer"),
            ("0.0.100-g000001", "0.0.100-gffffff", "current"),
            ("0.0.100-gffffff", "0.0.100", "current"),
            ("0.1.1", "0.0.999", "newer"),
        ] {
            assert_eq!(compare(current, latest).unwrap(), expected);
        }
        for invalid in ["", "dev", "amp-fixture 1.0", "0.0.x"] {
            assert!(compare(invalid, "0.0.100").is_err());
        }
    }

    async fn source(body: String) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route("/", get(move || async move { body })),
            )
            .await
            .unwrap();
        });
        (url, task)
    }

    #[tokio::test]
    async fn metadata_is_bounded_and_strict() {
        let client = reqwest::Client::new();
        for (body, valid) in [
            ("0.0.1789689675-g7fec46\n".into(), true),
            ("<html>unavailable</html>".into(), false),
            ("0.0.100 extra".into(), false),
            ("x".repeat(257), false),
        ] {
            let (url, server) = source(body).await;
            let result = latest_version(&client, &url).await;
            assert_eq!(result.is_ok(), valid);
            if valid {
                assert_eq!(result.unwrap(), "0.0.1789689675-g7fec46");
            }
            server.abort();
        }
    }

    #[test]
    fn wsl_update_keeps_distribution_path_and_argument_boundaries() {
        let mut runner = sample(std::path::Path::new("/home/me/project & space"));
        runner.backend = "wsl".into();
        runner.distribution = "Ubuntu Test".into();
        runner.executable = "/home/me/custom amp".into();
        let cmd = command(Some(&runner), &["update".into(), "--porcelain".into()]);
        assert_eq!(cmd.as_std().get_program(), "wsl.exe");
        assert_eq!(
            cmd.as_std().get_args().collect::<Vec<_>>(),
            [
                "--distribution",
                "Ubuntu Test",
                "--cd",
                "/home/me/project & space",
                "--exec",
                "/home/me/custom amp",
                "update",
                "--porcelain"
            ]
        );
    }

    #[tokio::test]
    async fn isolated_cli_check_update_failures_and_concurrency() {
        let directory = tempfile::tempdir().unwrap();
        let fixture = directory.path().join(if cfg!(windows) {
            "custom amp.exe"
        } else {
            "custom amp"
        });
        let compiled = std::process::Command::new("rustc")
            .args(["tests/fixtures/fake_amp.rs", "-o"])
            .arg(&fixture)
            .output()
            .unwrap();
        assert!(
            compiled.status.success(),
            "{}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        fs::write(fixture.with_extension("version"), "0.0.99-gffffff").unwrap();
        fs::write(fixture.with_extension("next"), "0.0.100-g000001").unwrap();
        let mut app = test_app(directory.path());
        Arc::get_mut(&mut app.cli).unwrap().client = reqwest::Client::new();
        let mut runner = sample(directory.path());
        runner.executable = fixture.to_string_lossy().into();
        let id = runner.id.as_str();
        assert_eq!(
            request(&app, "POST", "/api/runners", json!(runner)).await.0,
            StatusCode::OK
        );
        let (url, server) = source("0.0.100-g000001".into()).await;
        let cli = &app.cli;
        let state = cli.inspect(&app, id, Some(&url)).await.unwrap();
        assert_eq!(state.status, "available");
        assert_eq!(state.version.as_deref(), Some("0.0.99-gffffff"));
        assert!(
            !fixture.with_extension("args").exists(),
            "checking must not run update"
        );

        app.manager.lock().await.action(id, "start").await.unwrap();
        assert_eq!(
            cli.install(&app, id).await.unwrap_err().0,
            StatusCode::CONFLICT
        );
        assert!(
            app.manager.lock().await.active(id),
            "never stop a Runner for an update"
        );
        app.manager.lock().await.action(id, "stop").await.unwrap();
        app.manager.lock().await.config.runners[0].executable = "missing".into();
        assert!(
            cli.install(&app, id)
                .await
                .unwrap_err()
                .1
                .contains("已改变")
        );
        app.manager.lock().await.config.runners[0] = runner.clone();
        assert!(!fixture.with_extension("args").exists());

        fs::write(fixture.with_extension("behavior"), "wait").unwrap();
        let updating = {
            let app = app.clone();
            let id = id.to_owned();
            tokio::spawn(async move {
                request(&app, "POST", "/api/cli/update", json!({"runner_id": id})).await
            })
        };
        tokio::time::timeout(Duration::from_secs(10), async {
            while !fixture.with_extension("args").exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let (_, state) = tokio::time::timeout(
            Duration::from_secs(1),
            request(&app, "GET", "/api/state", json!(null)),
        )
        .await
        .unwrap();
        assert_eq!(state["cli"][id]["status"], "updating");
        assert_eq!(
            cli.install(&app, id).await.unwrap_err().0,
            StatusCode::CONFLICT
        );
        assert_eq!(
            app.updater.check().await.unwrap_err().0,
            StatusCode::CONFLICT
        );
        {
            let mut manager = app.manager.lock().await;
            assert_eq!(
                manager.action(id, "start").await.unwrap_err().0,
                StatusCode::CONFLICT
            );
            let config = manager.config.clone();
            assert_eq!(manager.save(config).unwrap_err().0, StatusCode::CONFLICT);
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(30), app.updater.finish())
                .await
                .is_err()
        );
        fs::write(fixture.with_extension("continue"), "").unwrap();
        let (status, result) = updating.await.unwrap();
        assert_eq!(status, StatusCode::OK, "{result}");
        assert_eq!(result["status"], "current");
        assert_eq!(result["version"], "0.0.100-g000001");
        assert_eq!(
            fs::read_to_string(fixture.with_extension("args")).unwrap(),
            "update\n--porcelain"
        );
        assert_eq!(
            std::path::PathBuf::from(fs::read_to_string(fixture.with_extension("cwd")).unwrap())
                .canonicalize()
                .unwrap(),
            directory.path().canonicalize().unwrap()
        );
        assert!(!app.manager.lock().await.cli_updating);
        assert!(
            cli.inspect(&app, id, None)
                .await
                .unwrap_err()
                .1
                .contains("退出")
        );
        app.updater.closing.store(false, Ordering::SeqCst);

        // Dropping the HTTP request must not cancel the owned installation task.
        fs::write(fixture.with_extension("version"), "0.0.99-gffffff").unwrap();
        fs::remove_file(fixture.with_extension("args")).unwrap();
        fs::remove_file(fixture.with_extension("continue")).unwrap();
        cli.inspect(&app, id, Some(&url)).await.unwrap();
        let disconnected = {
            let app = app.clone();
            let id = id.to_owned();
            tokio::spawn(async move {
                request(&app, "POST", "/api/cli/update", json!({"runner_id": id})).await
            })
        };
        tokio::time::timeout(Duration::from_secs(10), async {
            while !fixture.with_extension("args").exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        disconnected.abort();
        assert!(disconnected.await.unwrap_err().is_cancelled());
        fs::write(fixture.with_extension("continue"), "").unwrap();
        tokio::time::timeout(Duration::from_secs(10), async {
            while cli.states.lock().await[id].status == "updating" {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(cli.states.lock().await[id].status, "current");
        assert_eq!(installed(Some(&runner)).await.unwrap(), "0.0.100-g000001");
        assert!(!app.manager.lock().await.cli_updating);

        for (behavior, expected) in [
            ("fail", "fixture update denied"),
            ("unchanged", "仍是旧版本"),
        ] {
            fs::write(fixture.with_extension("version"), "0.0.99-gffffff").unwrap();
            fs::write(fixture.with_extension("behavior"), behavior).unwrap();
            cli.inspect(&app, id, Some(&url)).await.unwrap();
            assert!(
                cli.install(&app, id)
                    .await
                    .unwrap_err()
                    .1
                    .contains(expected)
            );
            assert_eq!(cli.states.lock().await[id].status, "error");
            assert!(!app.manager.lock().await.cli_updating);
        }
        cli.inspect(&app, id, Some(&url)).await.unwrap();
        fs::write(fixture.with_extension("version"), "0.0.101-g000001").unwrap();
        fs::remove_file(fixture.with_extension("args")).unwrap();
        assert_eq!(cli.install(&app, id).await.unwrap().status, "newer");
        assert!(
            !fixture.with_extension("args").exists(),
            "do not update a version already ahead"
        );

        let (bad_url, bad_server) = source("offline".into()).await;
        assert!(cli.inspect(&app, id, Some(&bad_url)).await.is_err());
        let failed = cli.states.lock().await[id].clone();
        assert!(failed.available);
        assert!(failed.latest.is_none());
        assert_eq!(failed.version.as_deref(), Some("0.0.101-g000001"));
        bad_server.abort();
        app.manager.lock().await.config.runners[0].executable =
            directory.path().join("missing").to_string_lossy().into();
        assert!(cli.inspect(&app, id, None).await.is_err());
        assert!(!cli.states.lock().await[id].available);
        assert_eq!(
            request(
                &app,
                "POST",
                "/api/cli/check",
                json!({"runner_id":"missing"})
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
        server.abort();
    }
}
