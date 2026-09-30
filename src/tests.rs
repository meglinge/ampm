use super::*;
use axum::{
    body::{Body, to_bytes},
    http::Request as HttpRequest,
};
use tower::ServiceExt;

pub(crate) fn sample(directory: &FsPath) -> Runner {
    Runner {
        id: "test-runner".into(),
        name: "测试环境".into(),
        executable: "amp".into(),
        backend: "native".into(),
        distribution: String::new(),
        directories: vec![directory.to_string_lossy().into()],
        discover: false,
        terminal: false,
        share: false,
    }
}
pub(crate) fn test_app(directory: &FsPath) -> App {
    App {
        manager: Arc::new(Mutex::new(Manager::load(directory).unwrap())),
        updater: Arc::new(updates::Updater::new(directory, false).unwrap()),
        cli: Arc::new(cli::Cli::new().unwrap()),
        token: "test-token".into(),
        port: 19380,
    }
}
pub(crate) async fn request(
    app: &App,
    method: &str,
    uri: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = router(app.clone())
        .oneshot(
            HttpRequest::builder()
                .method(method)
                .uri(uri)
                .header("host", "127.0.0.1:19380")
                .header("x-ampm-token", "test-token")
                .header("content-type", "application/json")
                .body(if method == "GET" {
                    Body::empty()
                } else {
                    Body::from(body.to_string())
                })
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[test]
fn validates_ids_and_directory_identity() {
    let directory = tempfile::tempdir().unwrap();
    let mut runner = sample(directory.path());
    for id in ["a", "Workstation-42", &"a".repeat(63)] {
        runner.id = id.into();
        assert!(validate(&runner).is_ok(), "{id}");
    }
    for id in [
        "",
        "-alpha",
        "alpha-",
        "a_b",
        "a b",
        "中文",
        &"a".repeat(64),
    ] {
        runner.id = id.into();
        assert!(validate(&runner).is_err(), "{id}");
    }
    runner.id = "valid".into();
    runner.directories.push(runner.directories[0].clone());
    assert!(validate(&runner).unwrap_err().1.contains("重复"));
    #[cfg(windows)]
    {
        runner.directories[1] = format!(
            "{}/",
            runner.directories[0].replace('\\', "/").to_uppercase()
        );
        assert!(validate(&runner).is_err());
    }
    runner.directories = vec![directory.path().join("different").to_string_lossy().into()];
    assert!(validate(&runner).is_ok());
    assert!(verify_native_paths(&runner).is_err());
    runner.directories.clear();
    assert!(validate(&runner).is_err());
    runner.directories = (0..201)
        .map(|i| {
            directory
                .path()
                .join(i.to_string())
                .to_string_lossy()
                .into()
        })
        .collect();
    assert!(validate(&runner).is_ok());
    runner
        .directories
        .push(directory.path().join("overflow").to_string_lossy().into());
    assert!(validate(&runner).is_err());
}

#[test]
fn startup_arguments_preserve_paths_and_explicitly_disable_terminal() {
    let directory = tempfile::tempdir().unwrap();
    let mut runner = sample(directory.path());
    runner
        .directories
        .extend(["D:\\Projects\\a & b".into(), "D:\\代码\\'quoted'".into()]);
    let args = process::startup_args(&runner);
    assert_eq!(
        process::command(&runner, &args, true)
            .as_std()
            .get_current_dir(),
        Some(directory.path())
    );
    assert_eq!(
        args,
        vec![
            "--no-tui",
            "--runner-id",
            "test-runner",
            "--no-color",
            "--no-remote-control-terminal",
            "--dir",
            "D:\\Projects\\a & b",
            "--dir",
            "D:\\代码\\'quoted'"
        ]
    );
    runner.discover = true;
    runner.terminal = true;
    runner.share = true;
    let args = process::startup_args(&runner);
    assert!(args.contains(&"--discover-dirs".into()));
    assert!(args.contains(&"--share".into()));
    assert!(args.contains(&"--remote-control-terminal".into()));
    assert!(!args.contains(&"--no-remote-control-terminal".into()));
    runner.backend = "wsl".into();
    runner.distribution = "Ubuntu".into();
    runner.directories[0] = "/home/me/work & notes".into();
    let command = process::command(&runner, &args, true);
    let actual: Vec<_> = command
        .as_std()
        .get_args()
        .map(|s| s.to_string_lossy().to_string())
        .collect();
    assert_eq!(
        &actual[..5],
        [
            "--distribution",
            "Ubuntu",
            "--cd",
            "/home/me/work & notes",
            "--exec"
        ]
    );
    assert_eq!(&actual[5..9], ["setsid", "--wait", "sh", "-c"]);
    assert!(!actual[9].contains(&runner.directories[0])); // No user input in the shell script.
    assert_eq!(actual[11], "amp"); // Never replace the Linux command with a Windows path.
}

#[cfg(windows)]
#[test]
fn windows_amp_resolution_preserves_path_priority_and_explicit_commands() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home with spaces 项目");
    let first = directory.path().join("first");
    let second = directory.path().join("second");
    for bin in [home.join("bin"), first.clone(), second.clone()] {
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("amp.exe"), b"resolver fixture").unwrap();
    }
    let path = std::env::join_paths([&first, &second]).unwrap();
    for name in ["amp", "AMP.EXE"] {
        assert_eq!(
            process::resolve_amp(name, Some(&path), Some(&home)),
            first.join("amp.exe")
        );
        assert_eq!(
            process::resolve_amp(name, None, Some(&home)),
            home.join("bin/amp.exe")
        );
    }
    fs::remove_file(first.join("amp.exe")).unwrap();
    fs::create_dir(first.join("amp.exe")).unwrap(); // A directory is not an executable.
    assert_eq!(
        process::resolve_amp("amp", Some(&path), Some(&home)),
        second.join("amp.exe")
    );
    for explicit in [
        "custom-amp",
        "amp.cmd",
        r".\amp.exe",
        r"C:\missing installation\amp.exe",
    ] {
        assert_eq!(
            process::resolve_amp(explicit, Some(&path), Some(&home)),
            PathBuf::from(explicit)
        );
    }
    assert_eq!(
        process::resolve_amp("amp", None, None),
        PathBuf::from("amp")
    );
}

#[cfg(windows)]
#[tokio::test]
async fn windows_amp_commands_work_with_stale_desktop_path() {
    if std::env::var_os("AMPM_RESOLUTION_TEST_CHILD").is_none() {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path().join("Amp home & 项目");
        fs::create_dir_all(home.join("bin")).unwrap();
        fs::create_dir_all(home.join("work")).unwrap();
        let compiled = std::process::Command::new("rustc")
            .args(["tests/fixtures/fake_amp.rs", "-o"])
            .arg(home.join("bin/amp.exe"))
            .output()
            .unwrap();
        assert!(compiled.status.success(), "{:?}", compiled);
        // Isolate environment changes in a child test process, never mutate the test
        // runner's global environment or invoke the user's real Amp installation.
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "tests::windows_amp_commands_work_with_stale_desktop_path",
                "--nocapture",
            ])
            .env("AMPM_RESOLUTION_TEST_CHILD", "1")
            .env("AMP_HOME", &home)
            .env("PATH", std::env::var_os("SystemRoot").unwrap())
            .current_dir(home.join("work"))
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        return;
    }

    let home = PathBuf::from(std::env::var_os("AMP_HOME").unwrap());
    assert!(
        tokio::process::Command::new("amp")
            .arg("--version")
            .output()
            .await
            .is_err(),
        "The old PATH-only lookup must fail in this reproduction"
    );
    let app = test_app(&home);
    let (status, cli) = request(&app, "GET", "/api/cli", json!(null)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cli["available"], true);
    assert_eq!(cli["version"], "amp-fixture 1.0 (offline test double)");
    assert_eq!(
        cli["executable"],
        home.join("bin")
            .join("amp.exe")
            .to_string_lossy()
            .into_owned()
    );
    let Json(found) = external().await.unwrap();
    assert!(
        found["output"]
            .as_str()
            .unwrap()
            .contains("\"runner\", \"list\"")
    );
    let mut runner = sample(&home.join("work"));
    let mut child = process::Running::spawn(&runner).await.unwrap();
    assert!(child.refresh());
    let inspected = process::output(process::command(
        &runner,
        &["runner".into(), "dirs".into(), "list".into()],
        false,
    ))
    .await
    .unwrap();
    assert!(inspected.contains("\"runner\", \"dirs\", \"list\""));
    child.stop(&runner).await.unwrap();
    assert!(child.child.try_wait().unwrap().is_some());

    runner.executable = home.join("missing/amp.exe").to_string_lossy().into_owned();
    let error = match process::Running::spawn(&runner).await {
        Ok(_) => panic!("An explicit missing path must not fall back to another Amp"),
        Err(error) => error,
    };
    assert!(error.1.contains(&runner.executable));
    assert!(error.1.contains("完整路径"));
}

#[test]
fn corrupt_and_future_configs_are_never_overwritten() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("config.json");
    for content in [
        "{ broken json",
        "{\"version\":99,\"runners\":[]}",
        "null",
        "[]",
        "{\"version\":\"1\",\"runners\":[]}",
        "{\"version\":null}",
    ] {
        fs::write(&file, content).unwrap();
        assert!(Manager::load(directory.path()).is_err());
        assert_eq!(fs::read_to_string(&file).unwrap(), content);
    }
}

#[test]
fn legacy_directories_merge_without_loss_or_implicit_disk_write() {
    let directory = tempfile::tempdir().unwrap();
    let original = directory
        .path()
        .join("first & 项目")
        .to_string_lossy()
        .into_owned();
    let other = directory
        .path()
        .join("second")
        .to_string_lossy()
        .into_owned();
    let file = directory.path().join("config.json");
    let mut legacy_runner = json!(sample(directory.path()));
    legacy_runner["cwd"] = json!(original);
    legacy_runner["directories"] = json!([other]);
    legacy_runner["discover"] = json!(true);
    for version in [Some(1), None] {
        let mut old = json!({"runners": [legacy_runner.clone()]});
        if let Some(version) = version {
            old["version"] = json!(version);
        }
        let bytes = serde_json::to_vec(&old).unwrap();
        fs::write(&file, &bytes).unwrap();
        let mut manager = Manager::load(directory.path()).unwrap();
        assert_eq!(manager.config.version, 2);
        assert_eq!(
            manager.config.runners[0].directories,
            [original.clone(), other.clone()]
        );
        assert!(manager.config.runners[0].discover);
        assert_eq!(fs::read(&file).unwrap(), bytes);
        manager.save(manager.config.clone()).unwrap();
        let saved: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
        assert!(saved["runners"][0].get("cwd").is_none());
        assert_eq!(saved["version"], 2);
        assert_eq!(
            Manager::load(directory.path()).unwrap().config.runners[0].directories,
            [original.clone(), other.clone()]
        );
    }
    legacy_runner["directories"] = json!([original]);
    let invalid = serde_json::to_vec(&json!({"version":1,"runners":[legacy_runner]})).unwrap();
    fs::write(&file, &invalid).unwrap();
    assert!(Manager::load(directory.path()).is_err());
    assert_eq!(fs::read(&file).unwrap(), invalid);
}

#[tokio::test]
async fn every_stopped_directory_can_be_removed_but_not_the_last() {
    let directory = tempfile::tempdir().unwrap();
    let app = test_app(directory.path());
    let first = directory.path().join("first & 项目");
    let second = directory.path().join("second");
    for path in [&first, &second] {
        fs::create_dir(path).unwrap();
    }
    let mut runner = sample(&first);
    runner
        .directories
        .push(second.to_string_lossy().into_owned());
    assert_eq!(
        request(&app, "POST", "/api/runners", json!(runner)).await.0,
        StatusCode::OK
    );
    let uri = "/api/runners/test-runner/directories";
    // Removing the former cwd must work exactly like removing any other entry.
    assert_eq!(
        request(&app, "POST", uri, json!({"action":"remove","path":first}))
            .await
            .0,
        StatusCode::OK
    );
    let loaded = Manager::load(directory.path()).unwrap();
    let remaining = &loaded.config.runners[0];
    assert_eq!(
        remaining.directories,
        [second.to_string_lossy().into_owned()]
    );
    assert_eq!(
        process::command(remaining, &process::startup_args(remaining), true)
            .as_std()
            .get_current_dir(),
        Some(second.as_path())
    );
    let bytes = fs::read(directory.path().join("config.json")).unwrap();
    assert_eq!(
        request(&app, "POST", uri, json!({"action":"remove","path":second}))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        fs::read(directory.path().join("config.json")).unwrap(),
        bytes
    );
    assert_eq!(
        request(&app, "POST", uri, json!({"action":"add","path":first}))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        request(&app, "POST", uri, json!({"action":"remove","path":first}))
            .await
            .0,
        StatusCode::OK
    );
    assert!(first.is_dir() && second.is_dir());
}

#[tokio::test]
async fn api_crud_is_persistent_and_case_insensitive_ids_conflict() {
    let directory = tempfile::tempdir().unwrap();
    let app = test_app(directory.path());
    let mut runner = sample(directory.path());
    assert_eq!(
        request(&app, "POST", "/api/runners", json!(runner)).await.0,
        StatusCode::OK
    );
    runner.id = "TEST-Runner".into();
    assert_eq!(
        request(&app, "POST", "/api/runners", json!(runner)).await.0,
        StatusCode::CONFLICT
    );
    runner.id = "renamed-runner".into();
    runner.name = "新的工作区".into();
    assert_eq!(
        request(&app, "PUT", "/api/runners/test-runner", json!(runner))
            .await
            .0,
        StatusCode::OK
    );
    let loaded = Manager::load(directory.path()).unwrap();
    assert_eq!(loaded.config.runners[0].id, "renamed-runner");
    assert_eq!(loaded.config.runners[0].name, "新的工作区");
    assert_eq!(
        request(&app, "DELETE", "/api/runners/test-runner", json!({}))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(&app, "DELETE", "/api/runners/renamed-runner", json!({}))
            .await
            .0,
        StatusCode::OK
    );
    assert!(
        Manager::load(directory.path())
            .unwrap()
            .config
            .runners
            .is_empty()
    );
    assert!(directory.path().is_dir());
}

#[tokio::test]
async fn security_rejects_rebinding_cross_origin_and_missing_token() {
    let directory = tempfile::tempdir().unwrap();
    let app = test_app(directory.path());
    for (host, origin, token, expected) in [
        (
            "evil.example:19380",
            "http://evil.example:19380",
            "test-token",
            StatusCode::FORBIDDEN,
        ),
        (
            "127.0.0.1:19380",
            "https://evil.example",
            "test-token",
            StatusCode::FORBIDDEN,
        ),
        (
            "127.0.0.1:19380",
            "http://127.0.0.1:19380",
            "",
            StatusCode::FORBIDDEN,
        ),
        (
            "127.0.0.1:19380",
            "http://127.0.0.1:19380",
            "test-token",
            StatusCode::OK,
        ),
    ] {
        let response = router(app.clone())
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/runners")
                    .header("host", host)
                    .header("origin", origin)
                    .header("x-ampm-token", token)
                    .header("content-type", "application/json")
                    .body(Body::from(json!(sample(directory.path())).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            expected,
            "host={host}, origin={origin}, token={token}"
        );
    }
}

#[tokio::test]
async fn directory_browser_lists_only_directories_and_preserves_unicode() {
    let directory = tempfile::tempdir().unwrap();
    fs::create_dir(directory.path().join("项目 a & b")).unwrap();
    fs::write(directory.path().join("not-a-directory.txt"), "test").unwrap();
    let Json(result) = browse(Query(Browse {
        path: Some(directory.path().to_string_lossy().into()),
    }))
    .await
    .unwrap();
    assert_eq!(result["entries"].as_array().unwrap().len(), 1);
    assert_eq!(result["entries"][0]["name"], "项目 a & b");
    assert!(
        browse(Query(Browse {
            path: Some("relative".into())
        }))
        .await
        .is_err()
    );
}

#[test]
fn log_buffer_retains_newest_entries_only() {
    let logs = process::Logs::default();
    for i in 0..305 {
        process::log(&logs, "stdout", format!("line-{i}"));
    }
    let lines = logs.lock().unwrap();
    assert_eq!(lines.len(), 300);
    assert_eq!(lines.front().unwrap().text, "line-5");
    assert_eq!(lines.back().unwrap().text, "line-304");
}

#[tokio::test]
async fn real_child_lifecycle_partial_batch_failure_and_live_directory_errors() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = directory.path().join(if cfg!(windows) {
        "fake-amp.exe"
    } else {
        "fake-amp"
    });
    let compilation = std::process::Command::new("rustc")
        .args(["tests/fixtures/fake_amp.rs", "-o"])
        .arg(&fixture)
        .output()
        .unwrap();
    assert!(
        compilation.status.success(),
        "{}",
        String::from_utf8_lossy(&compilation.stderr)
    );
    let app = test_app(directory.path());
    let mut runner = sample(directory.path());
    runner.executable = fixture.to_string_lossy().into();
    let initial = directory.path().join("initial dir");
    let live = directory.path().join("live & unicode 项目");
    let rejected = directory.path().join("rejected");
    for path in [&initial, &live, &rejected] {
        fs::create_dir(path).unwrap();
    }
    runner.directories.push(initial.to_string_lossy().into());
    assert_eq!(
        request(&app, "POST", "/api/runners", json!(runner)).await.0,
        StatusCode::OK
    );
    let (_, result) = request(
        &app,
        "POST",
        "/api/batch",
        json!({"ids":["test-runner", "missing", "test-runner"], "action":"start"}),
    )
    .await;
    assert_eq!(result["results"].as_array().unwrap().len(), 2);
    assert_eq!(result["results"][0]["ok"], true);
    assert_eq!(result["results"][1]["ok"], false);
    assert_eq!(
        request(
            &app,
            "POST",
            "/api/runners/test-runner/action",
            json!({"action":"start"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        request(&app, "PUT", "/api/runners/test-runner", json!(runner))
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        request(&app, "DELETE", "/api/runners/test-runner", json!({}))
            .await
            .0,
        StatusCode::CONFLICT
    );
    for path in &runner.directories {
        assert_eq!(
            request(
                &app,
                "POST",
                "/api/runners/test-runner/directories",
                json!({"action":"remove", "path":path})
            )
            .await
            .0,
            StatusCode::CONFLICT
        );
    }
    assert_eq!(
        request(
            &app,
            "POST",
            "/api/runners/test-runner/directories",
            json!({"action":"add", "path":live})
        )
        .await
        .0,
        StatusCode::OK
    );
    let (status, _) = request(
        &app,
        "POST",
        "/api/runners/test-runner/directories",
        json!({"action":"add", "path":rejected}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let loaded = Manager::load(directory.path()).unwrap();
    assert_eq!(
        loaded.config.runners[0].directories,
        [
            runner.directories[0].clone(),
            initial.to_string_lossy().into_owned(),
            live.to_string_lossy().into_owned()
        ]
    );
    assert!(
        !loaded.config.runners[0]
            .directories
            .iter()
            .any(|p| p.contains("rejected"))
    );
    assert_eq!(
        request(
            &app,
            "POST",
            "/api/runners/test-runner/directories",
            json!({"action":"remove", "path":live})
        )
        .await
        .0,
        StatusCode::OK
    );
    let (_, inspected) = request(&app, "POST", "/api/runners/test-runner/inspect", json!({})).await;
    assert!(
        inspected["output"]
            .as_str()
            .unwrap()
            .contains("--runner-id")
    );
    let (_, lines) = request(&app, "GET", "/api/runners/test-runner/logs", json!({})).await;
    assert!(
        lines["lines"]
            .as_array()
            .unwrap()
            .iter()
            .any(|line| line["text"].as_str().unwrap().contains("fixture stderr"))
    );
    assert_eq!(
        request(
            &app,
            "POST",
            "/api/runners/test-runner/action",
            json!({"action":"stop"})
        )
        .await
        .0,
        StatusCode::OK
    );
    let (_, state) = request(&app, "GET", "/api/state", json!({})).await;
    assert_eq!(state["runners"][0]["status"], "stopped");
    assert_eq!(state["runners"][0]["pid"], Value::Null);
    assert!(live.is_dir());
    assert!(initial.is_dir());
    runner.id = "exit-fixture".into();
    request(&app, "POST", "/api/runners", json!(runner)).await;
    request(
        &app,
        "POST",
        "/api/runners/exit-fixture/action",
        json!({"action":"start"}),
    )
    .await;
    for _ in 0..50 {
        if !app.manager.lock().await.active("exit-fixture") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let (_, state) = request(&app, "GET", "/api/state", json!({})).await;
    assert_eq!(state["runners"][1]["status"], "exited");

    // No GUI polling: the service itself must observe exits and stop live children on shutdown.
    app.manager
        .lock()
        .await
        .action("test-runner", "start")
        .await
        .unwrap();
    app.manager
        .lock()
        .await
        .action("exit-fixture", "start")
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (shutdown, stopped) = tokio::sync::oneshot::channel();
    let service = tokio::spawn(serve(app.clone(), listener, async {
        let _ = stopped.await;
    }));
    tokio::time::timeout(std::time::Duration::from_secs(6), async {
        loop {
            if app.manager.lock().await.processes["exit-fixture"]
                .exit
                .is_some()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("background monitor did not observe child exit");
    assert!(
        app.manager.lock().await.processes["test-runner"]
            .exit
            .is_none()
    );
    shutdown.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(6), service)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let mut manager = app.manager.lock().await;
    let child = manager.processes.get_mut("test-runner").unwrap();
    assert_eq!(child.exit.as_deref(), Some("由管理器停止"));
    assert!(child.child.try_wait().unwrap().is_some());
    assert!(tokio::net::TcpStream::connect(address).await.is_err());
}
