#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(windows)]
mod desktop;
mod process;
#[cfg(test)]
mod tests;
mod updates;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, Request, State},
    http::{HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::Write,
    path::{Path as FsPath, PathBuf},
    sync::Arc,
};
use tokio::sync::Mutex;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Runner {
    pub id: String,
    pub name: String,
    #[serde(default = "default_executable")]
    pub executable: String,
    #[serde(default = "default_backend")]
    pub backend: String,
    #[serde(default)]
    pub distribution: String,
    #[serde(default)]
    pub directories: Vec<String>,
    #[serde(default)]
    pub discover: bool,
    #[serde(default)]
    pub terminal: bool,
}
fn default_executable() -> String {
    "amp".into()
}
fn default_backend() -> String {
    "native".into()
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Config {
    #[serde(default = "schema_version")]
    version: u32,
    #[serde(default)]
    runners: Vec<Runner>,
}
fn schema_version() -> u32 {
    2
}

pub struct ApiError(StatusCode, String);
pub fn fail(message: impl Into<String>) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, message.into())
}
fn conflict(message: impl Into<String>) -> ApiError {
    ApiError(StatusCode::CONFLICT, message.into())
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error": self.1}))).into_response()
    }
}
impl std::fmt::Debug for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.0, self.1)
    }
}

fn validate(runner: &Runner) -> Result<(), ApiError> {
    if runner.id.is_empty()
        || runner.id.len() > 63
        || !runner
            .id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        || runner.id.starts_with('-')
        || runner.id.ends_with('-')
    {
        return Err(fail(
            "Runner ID 应为 1–63 位英文字母、数字或连字符，首尾不能为连字符",
        ));
    }
    if runner.name.trim().is_empty() || runner.name.chars().count() > 80 {
        return Err(fail("显示名称应为 1–80 个字符"));
    }
    if !["native", "wsl"].contains(&runner.backend.as_str()) {
        return Err(fail("未知运行环境"));
    }
    if runner.backend == "wsl" && !cfg!(windows) {
        return Err(fail("WSL 启动方式仅适用于 Windows"));
    }
    if runner.executable.trim().is_empty() || runner.executable.contains(['\0', '\r', '\n']) {
        return Err(fail("请输入 Amp 可执行文件路径，不要附加命令参数"));
    }
    if runner.distribution.contains(['\0', '\r', '\n']) {
        return Err(fail("无效的 WSL 发行版名称"));
    }
    if runner.directories.is_empty() || runner.directories.len() > 201 {
        return Err(fail("每个 Runner 请配置 1–201 个目录"));
    }
    let mut paths = HashSet::new();
    for path in &runner.directories {
        if path.contains(['\0', '\r', '\n']) || path.trim().is_empty() {
            return Err(fail("文件夹路径不能为空或包含换行符"));
        }
        if runner.backend == "wsl" {
            if !path.starts_with('/') {
                return Err(fail("WSL 文件夹必须使用 Linux 绝对路径，如 /home/me/code"));
            }
        } else if !FsPath::new(path).is_absolute() {
            return Err(fail(format!("请使用绝对路径：{path}")));
        }
        if !paths.insert(path_key(path, &runner.backend)) {
            return Err(fail(format!("文件夹重复：{path}")));
        }
    }
    Ok(())
}

fn path_key(path: &str, backend: &str) -> String {
    if cfg!(windows) && backend == "native" {
        path.replace('/', "\\")
            .trim_end_matches('\\')
            .to_lowercase()
    } else {
        path.trim_end_matches('/').to_string()
    }
}

fn verify_native_paths(runner: &Runner) -> Result<(), ApiError> {
    if runner.backend == "native" {
        for path in &runner.directories {
            if !FsPath::new(path).is_dir() {
                return Err(fail(format!("文件夹不存在或不可访问：{path}")));
            }
        }
    }
    Ok(())
}

struct Manager {
    config: Config,
    file: PathBuf,
    processes: HashMap<String, process::Running>,
    updating: bool,
}
impl Manager {
    fn load(directory: &FsPath) -> Result<Self, Box<dyn std::error::Error>> {
        fs::create_dir_all(directory)?;
        let file = directory.join("config.json");
        let config: Config = if file.exists() {
            let mut value: Value = serde_json::from_slice(&fs::read(&file)?)?;
            let fields = value.as_object_mut().ok_or("无效的配置对象")?;
            let version = match fields.get("version") {
                Some(version) => version.as_u64().ok_or("无效的配置版本")?,
                None => 1,
            };
            match version {
                1 => {
                    // Upgrade in memory only. Preserve the launch location (and Amp's
                    // remembered directories) by keeping the former cwd first.
                    if let Some(runners) = fields.get_mut("runners").and_then(Value::as_array_mut) {
                        for runner in runners {
                            let fields = runner.as_object_mut().ok_or("无效的 Runner 配置")?;
                            let cwd = fields.remove("cwd").ok_or("旧配置缺少 cwd")?;
                            fields
                                .entry("directories")
                                .or_insert_with(|| json!([]))
                                .as_array_mut()
                                .ok_or("无效的目录列表")?
                                .insert(0, cwd);
                        }
                    }
                    fields.insert("version".into(), json!(schema_version()));
                }
                2 => {}
                _ => return Err("配置版本不受支持；原文件未修改".into()),
            }
            serde_json::from_value(value)?
        } else {
            Config {
                version: schema_version(),
                runners: vec![],
            }
        };
        if config.version != schema_version() {
            return Err("配置版本不受支持；原文件未修改".into());
        }
        let mut ids = HashSet::new();
        for runner in &config.runners {
            validate(runner).map_err(|e| e.1)?;
            if !ids.insert(runner.id.to_lowercase()) {
                return Err("配置中存在重复 Runner ID；原文件未修改".into());
            }
        }
        Ok(Self {
            config,
            file,
            processes: HashMap::new(),
            updating: false,
        })
    }
    fn ensure_writable(&self) -> Result<(), ApiError> {
        if self.updating {
            Err(conflict("AMPM 正在更新，请等待重启"))
        } else {
            Ok(())
        }
    }
    fn save(&mut self, config: Config) -> Result<(), ApiError> {
        self.ensure_writable()?;
        let mut temp = tempfile::NamedTempFile::new_in(self.file.parent().unwrap())
            .map_err(|e| fail(e.to_string()))?;
        temp.write_all(&serde_json::to_vec_pretty(&config).map_err(|e| fail(e.to_string()))?)
            .map_err(|e| fail(e.to_string()))?;
        temp.as_file().sync_all().map_err(|e| fail(e.to_string()))?;
        temp.persist(&self.file)
            .map_err(|e| fail(format!("保存配置失败：{e}")))?;
        self.config = config;
        Ok(())
    }
    fn runner(&self, id: &str) -> Result<Runner, ApiError> {
        self.config
            .runners
            .iter()
            .find(|r| r.id == id)
            .cloned()
            .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "Runner 不存在".into()))
    }
    fn active(&mut self, id: &str) -> bool {
        self.processes.get_mut(id).is_some_and(|p| p.refresh())
    }
    async fn action(&mut self, id: &str, action: &str) -> Result<(), ApiError> {
        self.ensure_writable()?;
        let runner = self.runner(id)?;
        if !["start", "stop", "restart"].contains(&action) {
            return Err(fail("未知操作"));
        }
        if action == "start" && self.active(id) {
            return Err(conflict("Runner 已在运行，不会重复启动"));
        }
        if ["stop", "restart"].contains(&action)
            && let Some(process) = self.processes.get_mut(id)
        {
            process.stop(&runner).await?;
        }
        if ["start", "restart"].contains(&action) {
            verify_native_paths(&runner)?;
            let process = process::Running::spawn(&runner).await?;
            self.processes.insert(id.to_string(), process);
        }
        Ok(())
    }
    fn view(&mut self) -> Value {
        let runners = self.config.runners.clone().into_iter().map(|runner| {
            let active = self.active(&runner.id);
            let process = self.processes.get(&runner.id);
            json!({"config": runner, "status": if active { "running" } else if process.is_some_and(|p| p.exit.as_deref() != Some("由管理器停止")) { "exited" } else { "stopped" },
                "pid": process.filter(|_| active).and_then(|p| p.child.id()), "started": process.filter(|_| active).map(|p| p.started),
                "exit": process.and_then(|p| p.exit.clone()), "initial_directories": process.filter(|_| active).map(|p| &p.initial_directories) })
        }).collect::<Vec<_>>();
        json!({"runners": runners, "config_path": self.file, "platform": std::env::consts::OS})
    }
}

#[derive(Clone)]
struct App {
    manager: Arc<Mutex<Manager>>,
    updater: Arc<updates::Updater>,
    token: String,
    port: u16,
}

async fn guard(State(app): State<App>, request: Request, next: Next) -> Response {
    let host = request
        .headers()
        .get("host")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    if host != format!("127.0.0.1:{}", app.port) && host != format!("localhost:{}", app.port) {
        return ApiError(StatusCode::FORBIDDEN, "仅允许本机地址访问".into()).into_response();
    }
    if let Some(origin) = request.headers().get("origin")
        && origin.to_str().unwrap_or("") != format!("http://{host}")
    {
        return ApiError(StatusCode::FORBIDDEN, "禁止跨站访问".into()).into_response();
    }
    if request.method() != axum::http::Method::GET
        && request
            .headers()
            .get("x-ampm-token")
            .and_then(|h| h.to_str().ok())
            != Some(&app.token)
    {
        return ApiError(StatusCode::FORBIDDEN, "无效的本机会话，请刷新页面".into())
            .into_response();
    }
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response
        .headers_mut()
        .insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    response
        .headers_mut()
        .insert("cache-control", HeaderValue::from_static("no-store"));
    response.headers_mut().insert("content-security-policy", HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'"));
    response
}

fn router(app: App) -> Router {
    Router::new()
        .route(
            "/",
            get(|| async { Html(include_str!("../web/index.html")) }),
        )
        .route(
            "/app.js",
            get(|| async {
                (
                    [("content-type", "text/javascript; charset=utf-8")],
                    include_str!("../web/app.js"),
                )
            }),
        )
        .route(
            "/style.css",
            get(|| async {
                (
                    [("content-type", "text/css; charset=utf-8")],
                    include_str!("../web/style.css"),
                )
            }),
        )
        .route("/api/state", get(state))
        .route("/api/runners", post(create_runner))
        .route(
            "/api/runners/{id}",
            axum::routing::put(update_runner).delete(delete_runner),
        )
        .route("/api/runners/{id}/action", post(runner_action))
        .route("/api/runners/{id}/directories", post(directory_action))
        .route("/api/runners/{id}/inspect", post(inspect_directories))
        .route("/api/runners/{id}/logs", get(logs))
        .route("/api/batch", post(batch))
        .route("/api/browse", get(browse))
        .route("/api/cli", get(cli_info))
        .route("/api/external", get(external))
        .route("/api/update/check", post(updates::check))
        .route("/api/update/install", post(updates::install))
        .route("/api/update/preferences", post(updates::preferences))
        .layer(DefaultBodyLimit::max(128 * 1024))
        .layer(middleware::from_fn_with_state(app.clone(), guard))
        .with_state(app)
}

async fn state(State(app): State<App>) -> Json<Value> {
    let mut view = app.manager.lock().await.view();
    view["token"] = json!(app.token);
    view["update"] = json!(app.updater.state().await);
    Json(view)
}
async fn create_runner(
    State(app): State<App>,
    Json(runner): Json<Runner>,
) -> Result<Json<Value>, ApiError> {
    validate(&runner)?;
    verify_native_paths(&runner)?;
    let mut manager = app.manager.lock().await;
    if manager
        .config
        .runners
        .iter()
        .any(|r| r.id.eq_ignore_ascii_case(&runner.id))
    {
        return Err(conflict("Runner ID 已存在（不区分大小写）"));
    }
    let mut config = manager.config.clone();
    config.runners.push(runner);
    manager.save(config)?;
    Ok(Json(manager.view()))
}
async fn update_runner(
    State(app): State<App>,
    Path(id): Path<String>,
    Json(runner): Json<Runner>,
) -> Result<Json<Value>, ApiError> {
    validate(&runner)?;
    verify_native_paths(&runner)?;
    let mut manager = app.manager.lock().await;
    manager.runner(&id)?;
    if manager.active(&id) {
        return Err(conflict(
            "请先停止 Runner 再修改启动配置；运行中可单独添加目录",
        ));
    }
    if manager
        .config
        .runners
        .iter()
        .any(|r| r.id != id && r.id.eq_ignore_ascii_case(&runner.id))
    {
        return Err(conflict("Runner ID 已存在"));
    }
    let mut config = manager.config.clone();
    *config.runners.iter_mut().find(|r| r.id == id).unwrap() = runner;
    manager.save(config)?;
    manager.processes.remove(&id);
    Ok(Json(manager.view()))
}
async fn delete_runner(
    State(app): State<App>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let mut manager = app.manager.lock().await;
    manager.runner(&id)?;
    if manager.active(&id) {
        return Err(conflict("请先停止 Runner 再删除配置"));
    }
    let mut config = manager.config.clone();
    config.runners.retain(|r| r.id != id);
    manager.save(config)?;
    manager.processes.remove(&id);
    Ok(Json(manager.view()))
}
#[derive(Deserialize)]
struct Action {
    action: String,
}
async fn runner_action(
    State(app): State<App>,
    Path(id): Path<String>,
    Json(body): Json<Action>,
) -> Result<Json<Value>, ApiError> {
    let mut manager = app.manager.lock().await;
    manager.action(&id, &body.action).await?;
    Ok(Json(manager.view()))
}
#[derive(Deserialize)]
struct Batch {
    ids: Vec<String>,
    action: String,
}
async fn batch(State(app): State<App>, Json(body): Json<Batch>) -> Result<Json<Value>, ApiError> {
    if body.ids.is_empty() || body.ids.len() > 100 {
        return Err(fail("请选择 1–100 个 Runner"));
    }
    let mut manager = app.manager.lock().await;
    let mut results = vec![];
    let mut seen = HashSet::new();
    for id in body.ids {
        if !seen.insert(id.clone()) {
            continue;
        }
        let result = manager.action(&id, &body.action).await;
        results.push(json!({"id": id, "ok": result.is_ok(), "error": result.err().map(|e| e.1)}));
    }
    Ok(Json(json!({"results": results})))
}
#[derive(Deserialize)]
struct DirectoryAction {
    action: String,
    path: String,
}
async fn directory_action(
    State(app): State<App>,
    Path(id): Path<String>,
    Json(body): Json<DirectoryAction>,
) -> Result<Json<Value>, ApiError> {
    if !["add", "remove"].contains(&body.action.as_str()) {
        return Err(fail("未知目录操作"));
    }
    let mut manager = app.manager.lock().await;
    manager.ensure_writable()?;
    let runner = manager.runner(&id)?;
    let mut next_runner = runner.clone();
    if body.action == "add" {
        next_runner.directories.push(body.path.clone());
    } else {
        let key = path_key(&body.path, &runner.backend);
        if !runner
            .directories
            .iter()
            .any(|p| path_key(p, &runner.backend) == key)
        {
            return Err(fail("目录未配置"));
        }
        next_runner
            .directories
            .retain(|p| path_key(p, &runner.backend) != key);
    }
    validate(&next_runner)?;
    if body.action == "add" {
        verify_native_paths(&next_runner)?;
    }
    if manager.active(&id) {
        let process = manager.processes.get(&id).unwrap();
        if body.action == "remove"
            && process
                .initial_directories
                .iter()
                .any(|p| path_key(p, &runner.backend) == path_key(&body.path, &runner.backend))
        {
            return Err(conflict(
                "此目录在启动时提供，请先停止 Runner 再移除。运行中新增的目录可直接移除。",
            ));
        }
        let logs = process.logs.clone();
        let args = vec![
            "runner".into(),
            "dirs".into(),
            body.action.clone(),
            body.path.clone(),
            "--runner-id".into(),
            id.clone(),
        ];
        let text = process::output(process::command(&runner, &args, false)).await?;
        process::log(
            &logs,
            "manager",
            format!("目录 {}：{}\n{}", body.action, body.path, text.trim()),
        );
    }
    let mut config = manager.config.clone();
    *config.runners.iter_mut().find(|r| r.id == id).unwrap() = next_runner;
    manager
        .save(config)
        .map_err(|e| fail(format!("{}；若运行中目录已改变，请读取实际目录核对", e.1)))?;
    Ok(Json(manager.view()))
}
async fn inspect_directories(
    State(app): State<App>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let runner = {
        let mut manager = app.manager.lock().await;
        let runner = manager.runner(&id)?;
        if !manager.active(&id) {
            return Err(conflict("请先启动 Runner"));
        }
        runner
    };
    let args = vec![
        "runner".into(),
        "dirs".into(),
        "list".into(),
        "--runner-id".into(),
        id,
    ];
    Ok(Json(
        json!({"output": process::output(process::command(&runner, &args, false)).await?}),
    ))
}
async fn logs(State(app): State<App>, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    let manager = app.manager.lock().await;
    manager.runner(&id)?;
    let lines = manager
        .processes
        .get(&id)
        .map(|p| {
            p.logs
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(Json(json!({"lines": lines})))
}
#[derive(Deserialize)]
struct Browse {
    path: Option<String>,
}
async fn browse(Query(query): Query<Browse>) -> Result<Json<Value>, ApiError> {
    let path = query.path.unwrap_or_default();
    tokio::task::spawn_blocking(move || {
        if path.is_empty() && cfg!(windows) {
            let drives = ('A'..='Z').map(|c| format!("{c}:\\")).filter(|p| FsPath::new(p).is_dir()).map(|p| json!({"name": p, "path": p})).collect::<Vec<_>>();
            return Ok(Json(json!({"path": "", "parent": null, "entries": drives})));
        }
        let root = if path.is_empty() { PathBuf::from("/") } else { PathBuf::from(path) };
        if !root.is_absolute() { return Err(fail("请使用绝对路径")); }
        let mut entries = fs::read_dir(&root).map_err(|e| fail(format!("无法读取目录：{e}")))?
            .filter_map(Result::ok).filter(|e| e.path().is_dir()).map(|e| json!({"name": e.file_name().to_string_lossy(), "path": e.path().to_string_lossy()})).collect::<Vec<_>>();
        entries.sort_by_key(|e| e["name"].as_str().unwrap_or("").to_lowercase());
        Ok(Json(json!({"path": root.to_string_lossy(), "parent": root.parent().map(|p| p.to_string_lossy()), "entries": entries})))
    }).await.map_err(|e| fail(e.to_string()))?
}
async fn cli_info() -> Json<Value> {
    let mut cmd = process::native_command("amp");
    cmd.arg("--version").kill_on_drop(true);
    let executable = cmd.as_std().get_program().to_string_lossy().into_owned();
    match process::output(cmd).await {
        Ok(version) => {
            Json(json!({"available": true, "version": version.trim(), "executable": executable}))
        }
        Err(e) => Json(json!({"available": false, "error": e.1, "executable": executable})),
    }
}
async fn external() -> Result<Json<Value>, ApiError> {
    let mut cmd = process::native_command("amp");
    cmd.args(["runner", "list"]).kill_on_drop(true);
    Ok(Json(
        json!({"output": process::output(cmd).await?, "note": "Amp 原始输出（未假定 JSON 格式）。仅查看，不接管外部进程；旧版 CLI 可能不支持此命令。"}),
    ))
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("AMPM: {error}");
        #[cfg(windows)]
        if !std::env::args().any(|arg| arg == "--headless") {
            desktop::show_error(&error.to_string());
        }
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|arg| arg == "--version") {
        println!("AMPM {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let port = std::env::var("AMPM_PORT")
        .unwrap_or_else(|_| "19380".into())
        .parse::<u16>()?;
    let data = std::env::var_os("AMPM_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            directories::ProjectDirs::from("", "", "ampm")
                .expect("无法定位用户数据目录，请设置 AMPM_DATA_DIR")
                .data_local_dir()
                .to_path_buf()
        });
    fs::create_dir_all(&data)?;
    let lock = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(data.join("manager.lock"))?;
    lock.try_lock_exclusive()
        .map_err(|_| "该数据目录已有管理器运行，请点击系统托盘中的 AMPM 图标；如需独立实例，请使用其他 AMPM_DATA_DIR")?;
    let manager = Arc::new(Mutex::new(Manager::load(&data)?));
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    let port = listener.local_addr()?.port();
    let desktop = cfg!(windows) && !std::env::args().any(|arg| arg == "--headless");
    let updater = Arc::new(updates::Updater::new(&data, desktop)?);
    let app = App {
        manager,
        updater: updater.clone(),
        token: uuid::Uuid::new_v4().to_string(),
        port,
    };

    #[cfg(windows)]
    if desktop {
        let executable = std::env::current_exe()?;
        desktop::run(
            app,
            listener,
            &data,
            std::env::args().any(|arg| arg == "--open"),
        )
        .await?;
        // The new instance must not race the old instance's data lock or listening socket.
        drop(lock);
        if updater.restarting.load(std::sync::atomic::Ordering::SeqCst) {
            let mut command = std::process::Command::new(executable);
            command.args(std::env::args_os().skip(1).filter(|arg| arg != "--open"));
            if updater.visible.load(std::sync::atomic::Ordering::SeqCst) {
                command.arg("--open");
            }
            command
                .spawn()
                .map_err(|e| format!("新版已安装，但自动重启失败，请手动打开 AMPM：{e}"))?;
        }
        return Ok(());
    }

    println!("AMPM 后台服务：http://127.0.0.1:{port}\nCtrl+C 停止服务及托管进程。");
    serve(app, listener, std::future::pending()).await?;
    drop(lock);
    Ok(())
}

async fn serve(
    app: App,
    listener: tokio::net::TcpListener,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> Result<(), String> {
    let manager = app.manager.clone();
    let monitor_manager = manager.clone();
    // Process supervision must continue even when there is no visible GUI polling /api/state.
    let monitor = tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            for process in monitor_manager.lock().await.processes.values_mut() {
                process.refresh();
            }
        }
    });
    let result = axum::serve(listener, router(app))
        .with_graceful_shutdown(async move {
            tokio::select! {
                _ = shutdown => {},
                _ = tokio::signal::ctrl_c() => {},
            }
        })
        .await;
    monitor.abort();
    // Drain API requests before stopping children, so an in-flight start cannot escape cleanup.
    let mut manager = manager.lock().await;
    let mut errors = vec![];
    for runner in manager.config.runners.clone() {
        if let Some(process) = manager.processes.get_mut(&runner.id)
            && let Err(error) = process.stop(&runner).await
        {
            errors.push(format!("{}: {}", runner.id, error.1));
        }
    }
    result.map_err(|error| error.to_string())?;
    if !errors.is_empty() {
        return Err(format!(
            "部分 Runner 未能正常停止，请检查：\n{}",
            errors.join("\n")
        ));
    }
    Ok(())
}
