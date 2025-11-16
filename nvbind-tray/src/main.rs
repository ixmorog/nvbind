use anyhow::{Context, Result};
use ksni::blocking::TrayMethods;
use ksni::{self, Icon, ToolTip, menu};
use notify_rust::Notification;
use serde::Deserialize;
use std::collections::BTreeMap;
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
struct DeviceGroup {
    base: String,
    video: Option<Gpu>,
    audio: Option<Gpu>,
    others: Vec<Gpu>,
}

impl DeviceGroup {
    fn new(base: String) -> Self {
        Self {
            base,
            video: None,
            audio: None,
            others: vec![],
        }
    }

    fn all_bdfs(&self) -> Vec<String> {
        let mut bdfs = vec![];
        if let Some(v) = &self.video {
            bdfs.push(v.bdf.clone());
        }
        if let Some(a) = &self.audio {
            bdfs.push(a.bdf.clone());
        }
        for other in &self.others {
            bdfs.push(other.bdf.clone());
        }
        bdfs
    }
}

#[derive(Clone)]
struct Shared {
    status: Arc<Mutex<Status>>,
    tx: UnboundedSender<Command>,
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
        "NvBind".into()
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
            title: "NvBind".into(),
            description: "NVIDIA GPU Binding manager".into(),
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

        let groups = group_gpus(&st.gpus);
        if groups.is_empty() {
            items.push(
                menu::StandardItem {
                    label: "No NVIDIA GPU found".into(),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            );
        } else {
            for group in groups {
                let hdr = format_group_header(&group);
                items.push(
                    menu::StandardItem {
                        label: hdr,
                        enabled: false,
                        ..Default::default()
                    }
                    .into(),
                );

                let video_bdf = group.video.as_ref().map(|g| g.bdf.clone());
                let nvidia_targets = group.all_bdfs();
                let nvidia_targets_for_bind = nvidia_targets.clone();
                let vfio_targets = group.all_bdfs();
                let vfio_targets_for_bind = vfio_targets.clone();
                let vfio_targets_for_unbind = vfio_targets.clone();
                let has_video = video_bdf.is_some();
                let has_devices = !vfio_targets.is_empty();

                items.push(
                    menu::StandardItem {
                        label: format!(
                            "  → Bind to nvidia ({})",
                            video_bdf.as_deref().unwrap_or("n/a")
                        ),
                        enabled: has_video,
                        activate: Box::new(move |this: &mut MyTray| {
                            if nvidia_targets_for_bind.is_empty() {
                                return;
                            }
                            let _ = this
                                .shared
                                .tx
                                .send(Command::BindGroupToNvidia(nvidia_targets_for_bind.clone()));
                        }),
                        ..Default::default()
                    }
                    .into(),
                );

                items.push(
                    menu::StandardItem {
                        label: format!("  → Bind to vfio-pci ({})", format_bdf_list(&vfio_targets)),
                        enabled: has_devices,
                        activate: Box::new(move |this: &mut MyTray| {
                            if vfio_targets_for_bind.is_empty() {
                                return;
                            }
                            let _ = this
                                .shared
                                .tx
                                .send(Command::BindGroupToVfio(vfio_targets_for_bind.clone()));
                        }),
                        ..Default::default()
                    }
                    .into(),
                );

                items.push(
                    menu::StandardItem {
                        label: format!("  → Unbind ({})", format_bdf_list(&vfio_targets)),
                        enabled: has_devices,
                        activate: Box::new(move |this: &mut MyTray| {
                            if vfio_targets_for_unbind.is_empty() {
                                return;
                            }
                            let _ = this
                                .shared
                                .tx
                                .send(Command::UnbindGroup(vfio_targets_for_unbind.clone()));
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

fn group_gpus(gpus: &[Gpu]) -> Vec<DeviceGroup> {
    let mut groups: BTreeMap<String, DeviceGroup> = BTreeMap::new();
    for gpu in gpus {
        let (base, func) = split_bdf(&gpu.bdf);
        let entry = groups
            .entry(base.clone())
            .or_insert_with(|| DeviceGroup::new(base));
        match func.as_deref() {
            Some("0") | None => entry.video = Some(gpu.clone()),
            Some("1") => entry.audio = Some(gpu.clone()),
            _ => entry.others.push(gpu.clone()),
        }
    }
    groups.into_values().collect()
}

fn split_bdf(bdf: &str) -> (String, Option<String>) {
    match bdf.rsplit_once('.') {
        Some((base, func)) => (base.to_string(), Some(func.to_string())),
        None => (bdf.to_string(), None),
    }
}

fn format_group_header(group: &DeviceGroup) -> String {
    let mut parts = vec![
        format_device_or_placeholder(group.video.as_ref(), &group.base, "0", "video"),
        format_device_or_placeholder(group.audio.as_ref(), &group.base, "1", "audio"),
    ];
    parts.extend(group.others.iter().map(format_device));
    parts.join(" | ")
}

fn format_device(gpu: &Gpu) -> String {
    format!(
        "{} [{}:{}] [{}]",
        gpu.bdf,
        gpu.vendor,
        gpu.device,
        gpu.driver.clone().unwrap_or_else(|| "none".into())
    )
}

fn format_device_or_placeholder(gpu: Option<&Gpu>, base: &str, func: &str, label: &str) -> String {
    match gpu {
        Some(dev) => format_device(dev),
        None => format!("{}.{} [no {} function]", base, func, label),
    }
}

fn format_bdf_list(bdfs: &[String]) -> String {
    if bdfs.is_empty() {
        "n/a".into()
    } else {
        bdfs.join(", ")
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

async fn call_many_async(method: &str, bdfs: &[String]) -> Result<()> {
    for bdf in bdfs {
        call_method_async(method, bdf).await?;
    }
    Ok(())
}

// -------------------- Commands handled by the background worker --------------------

enum Command {
    Refresh,
    BindGroupToNvidia(Vec<String>),
    BindGroupToVfio(Vec<String>),
    UnbindGroup(Vec<String>),
}

fn main() -> Result<()> {
    // Channel that carries UI requests into the Tokio runtime.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Command>();

    // Events emitted from the worker to the UI thread.
    let (ux_tx, ux_rx) = std_mpsc::channel::<UiEvent>();

    // Initialize the StatusNotifierItem tray.
    let tray = MyTray::new(tx.clone());
    let handle = tray.spawn().context("spawn tray")?;

    // UI thread: only update the tray state and show notifications.
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
                        Notification::new().summary("NvBind").body(&msg).show().ok();
                    }
                    UiEvent::NotifyErr(msg) => {
                        Notification::new()
                            .summary("NvBind error")
                            .body(&msg)
                            .show()
                            .ok();
                    }
                }
            }
        });
    }

    // Async runtime: talks to D-Bus via zbus and forwards status updates.
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
                            Command::BindGroupToNvidia(bdfs) => {
                                match call_many_async("BindToNvidia", &bdfs).await {
                                    Ok(_) => {
                                        let msg = format!(
                                            "Bound {} to host drivers",
                                            format_bdf_list(&bdfs)
                                        );
                                        let _ = ux_tx.send(UiEvent::NotifyOk(msg));
                                        let _ = trigger_refresh(&ux_tx).await;
                                    }
                                    Err(e) => { let _ = ux_tx.send(UiEvent::NotifyErr(format!("{:#}", e))); }
                                }
                            }
                            Command::BindGroupToVfio(bdfs) => {
                                match call_many_async("BindToVfio", &bdfs).await {
                                    Ok(_) => {
                                        let msg = format!(
                                            "Bound {} to vfio-pci",
                                            format_bdf_list(&bdfs)
                                        );
                                        let _ = ux_tx.send(UiEvent::NotifyOk(msg));
                                        let _ = trigger_refresh(&ux_tx).await;
                                    }
                                    Err(e) => { let _ = ux_tx.send(UiEvent::NotifyErr(format!("{:#}", e))); }
                                }
                            }
                            Command::UnbindGroup(bdfs) => {
                                match call_many_async("Unbind", &bdfs).await {
                                    Ok(_) => {
                                        let msg = format!("Unbound {}", format_bdf_list(&bdfs));
                                        let _ = ux_tx.send(UiEvent::NotifyOk(msg));
                                        let _ = trigger_refresh(&ux_tx).await;
                                    }
                                    Err(e) => { let _ = ux_tx.send(UiEvent::NotifyErr(format!("{:#}", e))); }
                                }
                            }
                        }
                    }
                }
            }
        });
    });

    // Keep the main thread alive so the tray process does not exit early.
    loop {
        thread::sleep(Duration::from_secs(3600));
    }
}

async fn trigger_refresh(ux_tx: &std_mpsc::Sender<UiEvent>) -> Result<()> {
    let st = fetch_status_async().await?;
    let _ = ux_tx.send(UiEvent::SetStatus(st));
    Ok(())
}
