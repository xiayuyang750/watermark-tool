use std::io::Write;
use std::net::TcpStream;
use std::process::{Child, Command};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};

const BACKEND_PORT: u16 = 17890;

/// 后端端口已有进程监听则视为后端已在运行，避免重复启动。
fn backend_running() -> bool {
  TcpStream::connect(("127.0.0.1", BACKEND_PORT)).is_ok()
}

/// 请求已运行的后端优雅关闭（后端会顺带关闭常驻 Chromium，然后进程退出）。
fn request_backend_shutdown() {
  if let Ok(mut stream) = TcpStream::connect_timeout(
    &format!("127.0.0.1:{BACKEND_PORT}").parse().unwrap(),
    Duration::from_millis(500),
  ) {
    let _ = write!(
      stream,
      "POST /api/v1/shutdown HTTP/1.1\r\nHost: 127.0.0.1:{BACKEND_PORT}\r\nContent-Length: 0\r\n\r\n"
    );
    let _ = stream.flush();
  }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
  let backend: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
  let backend_setup = backend.clone();

  tauri::Builder::default()
    .setup(move |app| {
      // 端口被占用时（升级场景：旧版后端仍在运行），先请求旧后端优雅关闭，
      // 等待端口释放后再拉起新版——避免新版前端静默复用旧代码后端
      // （旧后端无残留清理逻辑，是每次升级出现 chrome failed to start 的根源）。
      if backend_running() {
        request_backend_shutdown();
        for _ in 0..30 {
          if !backend_running() {
            break;
          }
          std::thread::sleep(Duration::from_millis(100));
        }
      }
      // 启动随包分发的后端（sidecar）；端口被占用则复用已有实例
      if !backend_running() {
        let exe = app
          .path()
          .resource_dir()?
          .join("watermark-backend")
          .join("wm-backend.exe");
        if exe.exists() {
          let child = Command::new(exe)
            .env("WATERMARK_TOOL_SPAWNED", "1")
            .spawn()
            .ok();
          *backend_setup.lock().unwrap() = child;
        }
      }

      // WebView2 用户数据目录：优先 WATERMARK_TOOL_HOME（开发沙箱环境不可写用户目录），
      // 否则回退到系统 app data 目录。
      let webview_dir = match std::env::var("WATERMARK_TOOL_HOME") {
        Ok(home) => std::path::PathBuf::from(home),
        Err(_) => app.path().app_data_dir()?,
      };
      WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("Watermark Tool")
        .inner_size(1200.0, 800.0)
        .min_inner_size(900.0, 600.0)
        .data_directory(webview_dir)
        .build()?;
      Ok(())
    })
    .build(tauri::generate_context!())
    .expect("error while building tauri application")
    .run(move |_app_handle, event| {
      if let RunEvent::Exit = event {
        // 应用退出时关闭由本应用拉起的后端进程：
        // 先请求后端优雅关闭（顺带关闭常驻 Chromium），超时再强杀兜底，避免残留进程
        if let Some(mut child) = backend.lock().unwrap().take() {
          request_backend_shutdown();
          std::thread::sleep(Duration::from_millis(1500));
          if child.try_wait().ok().flatten().is_none() {
            let _ = child.kill();
          }
        }
      }
    });
}
