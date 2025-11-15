use anyhow::{Context, Result};
use ksni::blocking::TrayMethods;
use ksni::{self, menu, Icon, ToolTip};
use notify_rust::Notification;
use serde::Deserialize;
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;
use zbus::{Connection, Proxy};

const BUS_NAME: &str = "org.example.NvBind";
const OBJ_PATH: &str = "/org/example/NvBind";
const IFACE: &str = "org.example.NvBind";

enum UiEvent {
    SetStatus(Status),
    NotifyOk(String),
    NotifyErr(String),
}

#[derive(Debug, Deserialize, Clone)]
struct Gpu {
    bdf: String,
    vendor: String,
    device: String,
    driver: Option<String>,
}
#[derive(Debug, Deserialize, Clone, Default)]
struct Status {
    gpus: Vec<Gpu>,
}

#[derive(Clone)]
struct Shared {
    status: Arc<Mutex<Status>>,
    tx: UnboundedSender<Command>, // канал в наш фоновый async-рантайм
}

#[derive(Clone)]
struct MyTray {
    shared: Shared,
}

impl MyTray {
    fn new(tx: UnboundedSender<Command>) -> Self {
        Self {
            shared: Shared {
                status: Arc::new(Mutex::new(Status::default())),
                tx,
            },
        }
    }
}

impl ksni::Tray for MyTray {
    fn id(&self) -> String {
        "nvbind-tray".into()
    }
    fn title(&self) -> String {
        "NVBind".into()
    }
    fn icon_name(&self) -> String {
        "video-display".into()
    }
    fn icon_pixmap(&self) -> Vec<Icon> {
        vec![Icon {
            width: 1,
            height: 1,
            data: vec![0, 0, 0, 0],
        }]
    }
    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: "NVBind".into(),
            description: "NVIDIA GPU binding manager".into(),
            ..Default::default()
        }
    }

    fn menu(&self) -> Vec<menu::MenuItem<Self>> {
        let st = self.shared.status.lock().unwrap().clone();
        let mut items: Vec<menu::MenuItem<Self>> = vec![
            menu::StandardItem {
                label: "Refresh status".into(),
                activate: Box::new(|this: &mut MyTray| {
                    let _ = this.shared.tx.send(Command::Refresh);
                }),
                ..Default::default()
            }
            .into(),
            menu::MenuItem::Separator,
        ];

        if st.gpus.is_empty() {
            items.push(
                menu::StandardItem {
                    label: "No NVIDIA GPU found".into(),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            );
        } else {
            for g in st.gpus {
                let hdr = format!(
                    "{} [{}:{}] [{}]",
                    g.bdf,
                    g.vendor,
                    g.device,
                    g.driver.clone().unwrap_or_else(|| "none".into())
                );
                items.push(
                    menu::StandardItem {
                        label: hdr,
                        enabled: false,
                        ..Default::default()
                    }
                    .into(),
                );

                let bdf_nv = g.bdf.clone();
                items.push(
                    menu::StandardItem {
                        label: format!("  → Bind to nvidia ({})", bdf_nv),
                        activate: Box::new(move |this: &mut MyTray| {
                            let _ = this.shared.tx.send(Command::BindToNvidia(bdf_nv.clone()));
                        }),
                        ..Default::default()
                    }
                    .into(),
                );

                let bdf_vfio = g.bdf.clone();
                items.push(
                    menu::StandardItem {
                        label: format!("  → Bind to vfio-pci ({})", bdf_vfio),
                        activate: Box::new(move |this: &mut MyTray| {
                            let _ = this.shared.tx.send(Command::BindToVfio(bdf_vfio.clone()));
                        }),
                        ..Default::default()
                    }
                    .into(),
                );

                let bdf_un = g.bdf.clone();
                items.push(
                    menu::StandardItem {
                        label: format!("  → Unbind ({})", bdf_un),
                        activate: Box::new(move |this: &mut MyTray| {
                            let _ = this.shared.tx.send(Command::Unbind(bdf_un.clone()));
                        }),
                        ..Default::default()
                    }
                    .into(),
                );

                items.push(menu::MenuItem::Separator);
            }
        }

        items.push(
            menu::StandardItem {
                label: "Quit".into(),
                activate: Box::new(|_this| std::process::exit(0)),
                ..Default::default()
            }
            .into(),
        );

        items
    }
}

async fn fetch_status_async() -> Result<Status> {
    let conn = Connection::system().await.context("connect system dbus")?;
    let proxy = Proxy::new(&conn, BUS_NAME, OBJ_PATH, IFACE)
        .await
        .context("create proxy")?;
    let reply: String = proxy
        .call("GetStatus", &())
        .await
        .context("call GetStatus")?;
    let s: Status = serde_json::from_str(&reply).context("parse status json")?;
    Ok(s)
}

async fn call_method_async(method: &str, bdf: &str) -> Result<()> {
    let conn = Connection::system().await.context("connect system dbus")?;
    let proxy = Proxy::new(&conn, BUS_NAME, OBJ_PATH, IFACE)
        .await
        .context("create proxy")?;
    let _: () = proxy
        .call(method, &(bdf.to_string(),))
        .await
        .with_context(|| format!("call {}", method))?;
    Ok(())
}

// -------------------- Команды в фонового исполнителя --------------------

enum Command {
    Refresh,
    BindToNvidia(String),
    BindToVfio(String),
    Unbind(String),
}

fn main() -> Result<()> {
    // Канал команд в Async-рантайм (tokio)
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Command>();

    // Канал событий в UI-поток (std)
    let (ux_tx, ux_rx) = std_mpsc::channel::<UiEvent>();

    // Поднимаем ksni (трей)
    let tray = MyTray::new(tx.clone());
    let handle = tray.spawn().context("spawn tray")?;

    // UI-поток: только update() и notify-rust
    {
        let handle_ui = handle.clone();
        thread::spawn(move || {
            while let Ok(evt) = ux_rx.recv() {
                match evt {
                    UiEvent::SetStatus(st) => {
                        handle_ui.update(|my: &mut MyTray| {
                            my.shared.status.lock().unwrap().gpus = st.gpus.clone();
                        });
                    }
                    UiEvent::NotifyOk(msg) => {
                        Notification::new().summary("NVBind").body(&msg).show().ok();
                    }
                    UiEvent::NotifyErr(msg) => {
                        Notification::new()
                            .summary("NVBind error")
                            .body(&msg)
                            .show()
                            .ok();
                    }
                }
            }
        });
    }

    // Async-рантайм: только D-Bus (zbus) и отправка событий в UI
    thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .expect("tokio rt");
        rt.block_on(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(5));
            loop {
                tokio::select! {
                    _ = tick.tick() => {
                        match fetch_status_async().await {
                            Ok(st) => { let _ = ux_tx.send(UiEvent::SetStatus(st)); }
                            Err(e) => { let _ = ux_tx.send(UiEvent::NotifyErr(format!("{:#}", e))); }
                        }
                    }
                    Some(cmd) = rx.recv() => {
                        match cmd {
                            Command::Refresh => {
                                match fetch_status_async().await {
                                    Ok(st) => {
                                        let _ = ux_tx.send(UiEvent::SetStatus(st.clone()));
                                        if st.gpus.is_empty() {
                                            let _ = ux_tx.send(UiEvent::NotifyOk("No NVIDIA GPU found".into()));
                                        } else {
                                            let body = st.gpus.iter()
                                                .map(|g| format!("{} => {}", g.bdf, g.driver.clone().unwrap_or_else(|| "none".into())))
                                                .collect::<Vec<_>>().join("\n");
                                            let _ = ux_tx.send(UiEvent::NotifyOk(body));
                                        }
                                    }
                                    Err(e) => { let _ = ux_tx.send(UiEvent::NotifyErr(format!("{:#}", e))); }
                                }
                            }
                            Command::BindToNvidia(bdf) => {
                                match call_method_async("BindToNvidia", &bdf).await {
                                    Ok(_) => { let _ = ux_tx.send(UiEvent::NotifyOk("Bound to nvidia".into())); let _ = trigger_refresh(&ux_tx).await; }
                                    Err(e) => { let _ = ux_tx.send(UiEvent::NotifyErr(format!("{:#}", e))); }
                                }
                            }
                            Command::BindToVfio(bdf) => {
                                match call_method_async("BindToVfio", &bdf).await {
                                    Ok(_) => { let _ = ux_tx.send(UiEvent::NotifyOk("Bound to vfio-pci".into())); let _ = trigger_refresh(&ux_tx).await; }
                                    Err(e) => { let _ = ux_tx.send(UiEvent::NotifyErr(format!("{:#}", e))); }
                                }
                            }
                            Command::Unbind(bdf) => {
                                match call_method_async("Unbind", &bdf).await {
                                    Ok(_) => { let _ = ux_tx.send(UiEvent::NotifyOk("Unbound".into())); let _ = trigger_refresh(&ux_tx).await; }
                                    Err(e) => { let _ = ux_tx.send(UiEvent::NotifyErr(format!("{:#}", e))); }
                                }
                            }
                        }
                    }
                }
            }
        });
    });

    // держим главный поток живым
    loop {
        thread::sleep(Duration::from_secs(3600));
    }
}

async fn trigger_refresh(ux_tx: &std_mpsc::Sender<UiEvent>) -> Result<()> {
    let st = fetch_status_async().await?;
    let _ = ux_tx.send(UiEvent::SetStatus(st));
    Ok(())
}
