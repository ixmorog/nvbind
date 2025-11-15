use anyhow::Result;
use nvbind_core::{list_nvidia_gpus, bind_to_nvidia, bind_to_vfio, unbind};
use serde::{Deserialize, Serialize};
use tokio::signal;
use tracing::{info, error, Level};
use tracing_subscriber::EnvFilter;
use zbus::{ConnectionBuilder, interface, fdo};
use zbus::export::futures_util::TryFutureExt;

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
        Ok(serde_json::to_string(&super::Status { gpus }).unwrap())
    }

    async fn bind_to_nvidia(&self, bdf: &str) -> zbus::fdo::Result<()> {
        authorize("bind_to_nvidia")?;
        unbind(bdf).map_err(map_e)?;
        bind_to_nvidia(bdf).map_err(map_e)?;
        Ok(())
    }

    async fn bind_to_vfio(&self, bdf: &str) -> zbus::fdo::Result<()> {
        authorize("bind_to_vfio")?;
        unbind(bdf).map_err(map_e)?;
        bind_to_vfio(bdf).map_err(map_e)?;
        Ok(())
    }

    async fn unbind(&self, bdf: &str) -> zbus::fdo::Result<()> {
        authorize("unbind")?;
        unbind(bdf).map_err(map_e)?;
        Ok(())
    }
}

fn map_e(e: anyhow::Error) -> fdo::Error {
    fdo::Error::Failed(format!("{:#}", e))
}

fn authorize(_action: &str) -> Result<(), fdo::Error> {
    // Политику решает polkit (по конфигу D-Bus). Здесь можно расширить, если нужно.
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    // journald-friendly logging
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).with_target(false).init();

    info!("nvbindd starting…");

    let conn = ConnectionBuilder::system()?
        .name(BUS_NAME)?
        .serve_at(OBJ_PATH, NvBindIface)?
        .build()
        .await?;

    info!("D-Bus name acquired: {}", BUS_NAME);

    signal::ctrl_c().map_err(|e| anyhow::anyhow!(e)).await?;
    info!("nvbindd exiting");
    drop(conn);
    Ok(())
}
