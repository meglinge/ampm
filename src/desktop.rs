use std::{path::Path, sync::atomic::Ordering};
use tao::{
    dpi::{LogicalSize, PhysicalPosition, PhysicalSize},
    event::{Event, StartCause, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    platform::run_return::EventLoopExtRunReturn,
    window::{Window, WindowBuilder},
};
use tray_icon::{
    MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};
use windows_sys::Win32::{
    Foundation::POINT,
    Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint},
    UI::WindowsAndMessaging::{
        GetCursorPos, IDYES, MB_DEFBUTTON2, MB_ICONERROR, MB_ICONWARNING, MB_OK, MB_YESNO,
        MessageBoxW,
    },
};
use wry::{NewWindowResponse, WebContext, WebView, WebViewBuilder};

use crate::App;

enum UserEvent {
    Tray(TrayIconEvent),
    Menu(MenuEvent),
    Update,
    Stopped,
}

// Field order matters: the WebView must be destroyed before its context and host window.
struct Panel {
    webview: Option<WebView>,
    context: WebContext,
    window: Window,
    url: String,
    updater: std::sync::Arc<crate::updates::Updater>,
}

impl Panel {
    fn show(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if self.webview.is_none() {
            let origin = url::Url::parse(&self.url)?.origin();
            let updater = self.updater.clone();
            self.webview = Some(
                WebViewBuilder::new_with_web_context(&mut self.context)
                    .with_url(&self.url)
                    .with_background_color((248, 249, 251, 255))
                    .with_ipc_handler(move |request| match request.body().as_str() {
                        "editor-open" => updater.editing.store(true, Ordering::SeqCst),
                        "editor-closed" => updater.editing.store(false, Ordering::SeqCst),
                        _ => {}
                    })
                    .with_navigation_handler(move |candidate| {
                        if url::Url::parse(&candidate).is_ok_and(|url| url.origin() == origin) {
                            true
                        } else {
                            open_amp_link(&candidate);
                            false
                        }
                    })
                    .with_new_window_req_handler(|candidate, _| {
                        open_amp_link(&candidate);
                        NewWindowResponse::Deny
                    })
                    .build(&self.window)?,
            );
        }
        self.window.set_minimized(false);
        self.window.set_maximized(false);
        // Use the work area of the monitor containing the tray click, excluding its taskbar.
        unsafe {
            let mut cursor: POINT = std::mem::zeroed();
            GetCursorPos(&mut cursor);
            let monitor = MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST);
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(monitor, &mut info) != 0 {
                let work = info.rcWork;
                let width = (work.right - work.left) as u32;
                let height = (work.bottom - work.top) as u32;
                let outer = self.window.outer_size();
                if outer.width > width || outer.height > height {
                    let inner = self.window.inner_size();
                    self.window.set_inner_size(PhysicalSize::new(
                        inner
                            .width
                            .min(width.saturating_sub(outer.width - inner.width + 24)),
                        inner
                            .height
                            .min(height.saturating_sub(outer.height - inner.height + 24)),
                    ));
                }
                let size = self.window.outer_size();
                self.window.set_outer_position(PhysicalPosition::new(
                    work.left + (width.saturating_sub(size.width) / 2) as i32,
                    work.top + (height.saturating_sub(size.height) / 2) as i32,
                ));
            }
        }
        self.window.set_visible(true);
        self.window.set_focus();
        Ok(())
    }
}

fn is_amp_link(candidate: &str) -> bool {
    url::Url::parse(candidate).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str() == Some("ampcode.com")
            && url.port_or_known_default() == Some(443)
            && url.username().is_empty()
            && url.password().is_none()
    })
}

fn open_amp_link(candidate: &str) {
    if is_amp_link(candidate) {
        let _ = open::that_detached(candidate);
    }
}

fn message_box(title: &str, message: &str, flags: u32) -> i32 {
    let title = title.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let message = message.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            flags,
        )
    }
}

pub fn show_error(message: &str) {
    message_box("AMPM", message, MB_OK | MB_ICONERROR);
}

fn icon_rgba() -> Vec<u8> {
    let mut rgba = Vec::with_capacity(32 * 32 * 4);
    for y in 0..32 {
        for x in 0..32 {
            let rounded = (x as f32 - 15.5).abs().max(9.5) - 9.5;
            let vertical = (y as f32 - 15.5).abs().max(9.5) - 9.5;
            let opaque = rounded * rounded + vertical * vertical <= 36.0;
            let bolt = (6..18).contains(&y) && x >= 19 - (y - 6) * 3 / 4 && x <= 21 - (y - 6) / 3
                || (15..27).contains(&y) && x >= 13 - (y - 15) / 4 && x <= 23 - (y - 15);
            rgba.extend(if !opaque {
                [0, 0, 0, 0]
            } else if bolt {
                [255, 255, 255, 255]
            } else {
                [217, 98, 64, 255]
            });
        }
    }
    rgba
}

pub async fn run(
    app: App,
    listener: tokio::net::TcpListener,
    data: &Path,
    initially_visible: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let rgba = icon_rgba();
    let window = WindowBuilder::new()
        .with_title("AMPM · Runner 管理器")
        .with_inner_size(LogicalSize::new(1280.0, 820.0))
        .with_min_inner_size(LogicalSize::new(640.0, 460.0))
        .with_visible(false)
        .with_window_icon(Some(tao::window::Icon::from_rgba(rgba.clone(), 32, 32)?))
        .build(&event_loop)?;
    let mut panel = Panel {
        webview: None,
        context: WebContext::new(Some(data.join("webview"))),
        window,
        url: format!("http://127.0.0.1:{}", app.port),
        updater: app.updater.clone(),
    };
    let menu = Menu::new();
    let show = MenuItem::new("打开管理器", true, None);
    let quit = MenuItem::new("退出 AMPM…", true, None);
    menu.append_items(&[&show, &PredefinedMenuItem::separator(), &quit])?;
    let tray_builder = TrayIconBuilder::new()
        .with_tooltip("AMPM · 后台运行")
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_icon(tray_icon::Icon::from_rgba(rgba, 32, 32)?);

    let proxy = event_loop.create_proxy();
    TrayIconEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(UserEvent::Tray(event));
    }));
    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(UserEvent::Menu(event));
    }));
    let updater = app.updater.clone();
    let background = tokio::spawn(crate::updates::Updater::background(app.clone()));
    let proxy = event_loop.create_proxy();
    let restart_updater = updater.clone();
    let restart_event = tokio::spawn(async move {
        restart_updater.restart.notified().await;
        let _ = proxy.send_event(UserEvent::Update);
    });
    let (shutdown, stopped) = tokio::sync::oneshot::channel();
    let proxy = event_loop.create_proxy();
    let server = tokio::spawn(async move {
        let result = crate::serve(app, listener, async {
            let _ = stopped.await;
        })
        .await;
        let _ = proxy.send_event(UserEvent::Stopped);
        result
    });
    let mut shutdown = Some(shutdown);
    let mut tray_builder = Some(tray_builder);
    let mut tray = None;
    let mut startup_error = None;

    event_loop.run_return(|event, _, flow| {
        *flow = ControlFlow::Wait;
        let mut open = false;
        match event {
            Event::NewEvents(StartCause::Init) => {
                match tray_builder.take().unwrap().build() {
                    Ok(icon) => tray = Some(icon),
                    Err(error) => {
                        startup_error = Some(error);
                        if let Some(shutdown) = shutdown.take() { let _ = shutdown.send(()); }
                    }
                }
                open = initially_visible && startup_error.is_none();
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                panel.window.set_visible(false);
                updater.visible.store(false, Ordering::SeqCst);
            }
            Event::UserEvent(UserEvent::Tray(TrayIconEvent::Click {
                button: MouseButton::Left, button_state: MouseButtonState::Up, ..
            })) => open = true,
            Event::UserEvent(UserEvent::Menu(event)) if event.id == show.id() => open = true,
            Event::UserEvent(UserEvent::Menu(event)) if event.id == quit.id() => {
                if shutdown.is_some() && message_box(
                    "退出 AMPM？",
                    "退出会停止后台服务及 AMPM 托管的 Runner，正在执行的任务可能中断。\n\n只想收起界面，请关闭窗口。确定退出？",
                    MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
                ) == IDYES {
                    updater.closing.store(true, Ordering::SeqCst);
                    show.set_enabled(false);
                    quit.set_enabled(false);
                    panel.window.set_visible(false);
                    if let Some(tray) = &tray { let _ = tray.set_tooltip(Some("AMPM · 正在停止 Runner…")); }
                    if let Some(shutdown) = shutdown.take() { let _ = shutdown.send(()); }
                }
            }
            Event::UserEvent(UserEvent::Update) => {
                show.set_enabled(false);
                quit.set_enabled(false);
                panel.window.set_visible(false);
                if let Some(tray) = &tray { let _ = tray.set_tooltip(Some("AMPM · 正在更新重启…")); }
                if let Some(shutdown) = shutdown.take() { let _ = shutdown.send(()); }
            }
            Event::UserEvent(UserEvent::Stopped) => *flow = ControlFlow::Exit,
            _ => {}
        }
        if open && shutdown.is_some() && !updater.restarting.load(Ordering::SeqCst) {
            updater.visible.store(true, Ordering::SeqCst);
            if updater.restarting.load(Ordering::SeqCst) {
                updater.visible.store(false, Ordering::SeqCst);
                return;
            }
            if let Err(error) = panel.show() {
                updater.visible.store(false, Ordering::SeqCst);
                show_error(&format!("无法打开管理界面：{error}\n\n请检查 Microsoft Edge WebView2 Runtime 是否已安装。后台与托盘仍在运行。"));
            }
        }
    });
    if let Some(shutdown) = shutdown {
        let _ = shutdown.send(());
    }
    updater.finish().await;
    background.abort();
    restart_event.abort();
    server.await??;
    if let Some(error) = startup_error {
        return Err(error.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_official_https_links_can_leave_the_webview() {
        for allowed in [
            "https://ampcode.com",
            "https://ampcode.com/docs/cli/runners#usage",
        ] {
            assert!(is_amp_link(allowed));
        }
        for blocked in [
            "http://ampcode.com",
            "https://ampcode.com.evil.test",
            "https://ampcode.com@evil.test",
            "https://user@ampcode.com",
            "https://ampcode.com:9443",
            "file:///C:/Windows",
            "javascript:alert(1)",
            "ms-settings:privacy",
            "https://evil.test/?next=https://ampcode.com",
        ] {
            assert!(!is_amp_link(blocked), "{blocked}");
        }
    }
}
