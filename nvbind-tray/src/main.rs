use anyhow::{Context, Result};
use gtk::prelude::*;
use gtk::{Menu, MenuItem};
use libappindicator::{AppIndicator, AppIndicatorStatus};
use notify_rust::Notification;
use serde::Deserialize;
use std::rc::Rc;
use zbus::{blocking::Connection, zvariant::Value};

const BUS_NAME: &str = "org.example.NvBind";
const OBJ_PATH: &str = "/org/example/NvBind";
const IFACE: &str = "org.example.NvBind";

#[derive(Debug, Deserialize)]
struct Gpu { bdf: String, vendor: String, device: String, driver: Option<String> }
#[derive(Debug, Deserialize)]
struct Status { gpus: Vec<Gpu> }

fn main() -> Result<()> {
    gtk::init().context("init gtk")?;
    let app = AppIndicator::new("nvbind-tray", "video-display");
    app.set_status(AppIndicatorStatus::Active);
    app.set_title("NVBind");

    let conn = Rc::new(Connection::system().context("dbus system")?);

    let menu = build_menu(conn.clone(), &app)?;
    app.set_menu(&menu);

    gtk::main();
    Ok(())
}

fn build_menu(conn: Rc<Connection>, ind: &AppIndicator) -> Result<Menu> {
    let menu = Menu::new();

    let status = MenuItem::with_label("Refresh status");
    {
        let c = conn.clone();
        status.connect_activate(move |_| {
            if let Err(e) = show_status(&c) {
                eprintln!("status error: {e:#}");
            }
        });
    }
    menu.append(&status);

    let s = fetch_status(&conn)?;
    if s.gpus.is_empty() {
        let none = MenuItem::with_label("No NVIDIA GPU found");
        menu.append(&none);
    } else {
        for gpu in s.gpus {
            let label = format!("{} [{}:{}] [{}]", gpu.bdf, gpu.vendor, gpu.device, gpu.driver.clone().unwrap_or("none".into()));
            let item = MenuItem::with_label(&label);
            menu.append(&item);

            let to_nv = MenuItem::with_label(&format!("  → Bind to nvidia ({})", gpu.bdf));
            {
                let c = conn.clone();
                let bdf = gpu.bdf.clone();
                to_nv.connect_activate(move |_| {
                    call(&c, "bind_to_nvidia", &bdf, "Bound to nvidia");
                });
            }
            menu.append(&to_nv);

            let to_vfio = MenuItem::with_label(&format!("  → Bind to vfio-pci ({})", gpu.bdf));
            {
                let c = conn.clone();
                let bdf = gpu.bdf.clone();
                to_vfio.connect_activate(move |_| {
                    call(&c, "bind_to_vfio", &bdf, "Bound to vfio-pci");
                });
            }
            menu.append(&to_vfio);

            let unb = MenuItem::with_label(&format!("  → Unbind ({})", gpu.bdf));
            {
                let c = conn.clone();
                let bdf = gpu.bdf.clone();
                unb.connect_activate(move |_| {
                    call(&c, "unbind", &bdf, "Unbound");
                });
            }
            menu.append(&unb);
        }
    }

    let quit = MenuItem::with_label("Quit");
    quit.connect_activate(|_| gtk::main_quit());
    menu.append(&quit);

    menu.show_all();
    show_status(&conn)?;
    Ok(menu)
}

fn fetch_status(conn: &Connection) -> Result<Status> {
    let reply: String = conn.call_method(BUS_NAME, OBJ_PATH, IFACE, "get_status", &())?;
    let s: Status = serde_json::from_str(&reply)?;
    Ok(s)
}

fn show_status(conn: &Connection) -> Result<()> {
    let s = fetch_status(conn)?;
    let mut lines = vec![];
    if s.gpus.is_empty() {
        lines.push("No NVIDIA GPU found".into());
    } else {
        for g in s.gpus {
            lines.push(format!("{} => {}", g.bdf, g.driver.unwrap_or("none".into())));
        }
    }
    Notification::new()
        .summary("NVBind status")
        .body(&lines.join("\n"))
        .show().ok();
    Ok(())
}

fn call(conn: &Connection, method: &str, bdf: &str, ok_text: &str) {
    let r = conn.call_method(BUS_NAME, OBJ_PATH, IFACE, method, &(bdf.to_string(),));
    match r {
        Ok(_) => {
            Notification::new().summary("NVBind").body(ok_text).show().ok();
        }
        Err(e) => {
            Notification::new().summary("NVBind error").body(&format!("{e:#}")).show().ok();
        }
    }
}
