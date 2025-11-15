mod polkit;

use anyhow::Result;
use nvbind_core::{bind_to_nvidia, bind_to_vfio, list_nvidia_gpus, unbind};
use serde::{Deserialize, Serialize};
use tokio::signal;
use tracing::info;
use tracing_subscriber::EnvFilter;
use zbus::message::Header;
use zbus::{fdo, interface, Connection};

const BUS_NAME: &str = "org.example.NvBind";
const OBJ_PATH: &str = "/org/example/NvBind";

#[derive(Serialize, Deserialize, Debug)]
struct Status {
    gpus: Vec<nvbind_core::Gpu>,
}

struct NvBindIface;

#[interface(name = "org.example.NvBind")]
impl NvBindIface {
    async fn get_status(&self) -> zbus::fdo::Result<String> {
        let gpus = list_nvidia_gpus().map_err(map_e)?;
        Ok(serde_json::to_string(&Status { gpus }).unwrap())
    }
    async fn bind_to_nvidia(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(connection)] conn: &Connection,
        bdf: &str,
    ) -> fdo::Result<()> {
        let sender = hdr
            .sender()
            .ok_or_else(|| fdo::Error::Failed("no sender".into()))?;
        polkit::check_authz(conn, sender.as_str())
            .await
            .map_err(map_e)?;
        unbind(bdf).map_err(map_e)?;
        bind_to_nvidia(bdf).map_err(map_e)?;
        Ok(())
    }

    async fn bind_to_vfio(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(connection)] conn: &Connection,
        bdf: &str,
    ) -> fdo::Result<()> {
        let sender = hdr
            .sender()
            .ok_or_else(|| fdo::Error::Failed("no sender".into()))?;
        polkit::check_authz(conn, sender.as_str())
            .await
            .map_err(map_e)?;
        unbind(bdf).map_err(map_e)?;
        bind_to_vfio(bdf).map_err(map_e)?;
        Ok(())
    }

    async fn unbind(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(connection)] conn: &Connection,
        bdf: &str,
    ) -> fdo::Result<()> {
        let sender = hdr
            .sender()
            .ok_or_else(|| fdo::Error::Failed("no sender".into()))?;
        polkit::check_authz(conn, sender.as_str())
            .await
            .map_err(map_e)?;
        unbind(bdf).map_err(map_e)?;
        Ok(())
    }
}

fn map_e(e: anyhow::Error) -> fdo::Error {
    fdo::Error::Failed(format!("{:#}", e))
}

#[tokio::main]
async fn main() -> Result<()> {
    // journald-friendly logging
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();

    info!("nvbindd starting…");

    // Соединение с системным шиной
    let conn = Connection::system().await?;

    // Регистрируем объект
    conn.object_server().at(OBJ_PATH, NvBindIface).await?;
    // Запрашиваем имя
    conn.request_name(BUS_NAME).await?;

    info!("D-Bus name acquired: {}", BUS_NAME);

    // Блокируемся до Ctrl+C / stop
    signal::ctrl_c().await?;
    info!("nvbindd exiting");
    Ok(())
}
