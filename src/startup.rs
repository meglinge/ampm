use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::{ApiError, App, conflict, fail};

pub struct Startup {
    supported: bool,
    #[cfg(windows)]
    key: String,
    #[cfg(windows)]
    command: String,
}

#[derive(Debug, Serialize)]
pub struct StartupState {
    supported: bool,
    enabled: bool,
    error: Option<String>,
}

#[derive(Deserialize)]
pub struct Preferences {
    enabled: bool,
}

impl Startup {
    pub fn new(data: &Path, port: u16, desktop: bool) -> std::io::Result<Self> {
        #[cfg(not(windows))]
        let _ = (data, port);
        Ok(Self {
            supported: cfg!(windows) && desktop,
            #[cfg(windows)]
            key: r"Software\Microsoft\Windows\CurrentVersion\Run".into(),
            #[cfg(windows)]
            // Appending '.' avoids a trailing backslash escaping the closing quote.
            command: format!(
                "\"{}\" --data-dir \"{}\" --port {port}",
                std::env::current_exe()?.display(),
                data.join(".").display(),
            ),
        })
    }

    pub fn state(&self) -> StartupState {
        #[allow(unused_mut)] // The registry-backed fields are only updated on Windows.
        let mut state = StartupState {
            supported: self.supported,
            enabled: false,
            error: None,
        };
        #[cfg(windows)]
        if self.supported {
            match self.read() {
                Ok(value) => state.enabled = value.as_deref() == Some(&self.command),
                Err(error) => state.error = Some(format!("无法读取登录自启：{error}")),
            }
        }
        state
    }

    fn set(&self, enabled: bool) -> Result<StartupState, ApiError> {
        if !self.supported {
            return Err(fail("登录自启仅支持 Windows 桌面模式"));
        }
        #[cfg(windows)]
        {
            if enabled && self.command.encode_utf16().count() > 260 {
                return Err(fail(
                    "登录自启命令超过 Windows 的 260 字符限制，请缩短程序或数据目录路径",
                ));
            }
            if !enabled
                && self
                    .read()
                    .map_err(|e| fail(e.to_string()))?
                    .is_some_and(|value| value != self.command)
            {
                return Err(conflict("登录自启指向另一个 AMPM 实例，请在对应实例中关闭"));
            }
            self.write(enabled)
                .map_err(|e| fail(format!("无法设置登录自启：{e}")))?;
        }
        #[cfg(not(windows))]
        let _ = enabled;
        Ok(self.state())
    }
}

pub async fn preferences(
    State(app): State<App>,
    Json(body): Json<Preferences>,
) -> Result<Json<StartupState>, ApiError> {
    let _operation = app
        .updater
        .operation
        .try_lock()
        .map_err(|_| conflict("更新或退出正在进行，请稍后重试"))?;
    app.manager.lock().await.ensure_writable()?;
    if app
        .updater
        .closing
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        return Err(conflict("AMPM 正在退出"));
    }
    app.startup.set(body.enabled).map(Json)
}

#[cfg(windows)]
impl Startup {
    fn read(&self) -> std::io::Result<Option<String>> {
        use windows_sys::Win32::{Foundation::ERROR_FILE_NOT_FOUND, System::Registry::*};
        let key: Vec<u16> = self.key.encode_utf16().chain(Some(0)).collect();
        let mut bytes = 0;
        let result = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                windows_sys::w!("AMPM"),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut bytes,
            )
        };
        if result == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if result != 0 {
            return Err(std::io::Error::from_raw_os_error(result as i32));
        }
        let mut value = vec![0u16; (bytes as usize).div_ceil(2)];
        let result = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                windows_sys::w!("AMPM"),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                value.as_mut_ptr().cast(),
                &mut bytes,
            )
        };
        if result != 0 {
            return Err(std::io::Error::from_raw_os_error(result as i32));
        }
        value.truncate(bytes as usize / 2);
        while value.last() == Some(&0) {
            value.pop();
        }
        String::from_utf16(&value)
            .map(Some)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }

    fn write(&self, enabled: bool) -> std::io::Result<()> {
        use windows_sys::Win32::{Foundation::ERROR_FILE_NOT_FOUND, System::Registry::*};
        let key: Vec<u16> = self.key.encode_utf16().chain(Some(0)).collect();
        let result = if enabled {
            let mut handle = std::ptr::null_mut();
            let result = unsafe {
                RegCreateKeyExW(
                    HKEY_CURRENT_USER,
                    key.as_ptr(),
                    0,
                    std::ptr::null(),
                    REG_OPTION_NON_VOLATILE,
                    KEY_SET_VALUE,
                    std::ptr::null(),
                    &mut handle,
                    std::ptr::null_mut(),
                )
            };
            if result != 0 {
                return Err(std::io::Error::from_raw_os_error(result as i32));
            }
            let value: Vec<u16> = self.command.encode_utf16().chain(Some(0)).collect();
            unsafe {
                let result = RegSetValueExW(
                    handle,
                    windows_sys::w!("AMPM"),
                    0,
                    REG_SZ,
                    value.as_ptr().cast(),
                    (value.len() * 2) as u32,
                );
                RegCloseKey(handle);
                result
            }
        } else {
            unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), windows_sys::w!("AMPM")) }
        };
        if result == 0 || (!enabled && result == ERROR_FILE_NOT_FOUND) {
            Ok(())
        } else {
            Err(std::io::Error::from_raw_os_error(result as i32))
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn login_startup_roundtrips_without_touching_real_run_key() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = crate::tests::test_app(directory.path());
        let key = format!(r"Software\AMPM\Tests\{}", uuid::Uuid::new_v4());
        let data = directory.path().join("Data & 项目");
        let mut startup = Startup::new(&data, 19381, true).unwrap();
        startup.key = key.clone();
        let command = format!(
            "\"{}\" --data-dir \"{}\\.\" --port 19381",
            std::env::current_exe().unwrap().display(),
            data.display()
        );
        app.startup = std::sync::Arc::new(startup);
        for enabled in [true, true, false, false] {
            let (status, state) = crate::tests::request(
                &app,
                "POST",
                "/api/startup",
                serde_json::json!({"enabled":enabled}),
            )
            .await;
            assert_eq!(status, axum::http::StatusCode::OK, "{state}");
            assert_eq!(state["enabled"], enabled);
            assert_eq!(
                app.startup.read().unwrap().as_deref(),
                enabled.then_some(command.as_str())
            );
        }
        let mut other = Startup::new(&data, 19382, true).unwrap();
        other.key = key.clone();
        other.write(true).unwrap();
        assert_eq!(
            app.startup.set(false).unwrap_err().0,
            axum::http::StatusCode::CONFLICT
        );
        assert_eq!(
            other.read().unwrap().as_deref(),
            Some(other.command.as_str())
        );
        other.write(false).unwrap();
        let key: Vec<u16> = key.encode_utf16().chain(Some(0)).collect();
        unsafe {
            windows_sys::Win32::System::Registry::RegDeleteKeyW(
                windows_sys::Win32::System::Registry::HKEY_CURRENT_USER,
                key.as_ptr(),
            );
        }
    }
}
