use std::sync::Mutex;

#[cfg(not(target_os = "linux"))]
compile_error!("Roon Beacon Desktop supports Linux only");
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

fn main() {
    let app = tauri::Builder::default()
        .setup(|app| {
            let directory = app.path().app_data_dir()?;
            let server = tauri::async_runtime::block_on(
                roon_beacon::desktop::DesktopServer::start(directory),
            )?;
            let address = server.address();
            let url = server.launch_url().parse()?;
            let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
                .title("Roon Beacon")
                .inner_size(1180.0, 820.0)
                .min_inner_size(360.0, 600.0)
                .on_navigation(move |url| {
                    url.scheme() == "http"
                        && url.host_str() == Some("127.0.0.1")
                        && url.port() == Some(address.port())
                })
                .build();
            if let Err(error) = window {
                tauri::async_runtime::block_on(server.shutdown());
                return Err(error.into());
            }
            app.manage(Mutex::new(Some(server)));
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let (Ok(mut terminate), Ok(mut interrupt)) = (
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()),
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt()),
                ) {
                    tokio::select! {
                        _ = terminate.recv() => {},
                        _ = interrupt.recv() => {},
                    }
                    handle.exit(0);
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Cannot start Roon Beacon desktop");
    app.run(|app, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            if let Some(server) = app
                .state::<Mutex<Option<roon_beacon::desktop::DesktopServer>>>()
                .lock()
                .unwrap()
                .take()
            {
                tauri::async_runtime::block_on(server.shutdown());
            }
        }
    });
}
