use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::ErrorKind;
use std::{
    fs,
    path::{Path, PathBuf},
};
use tracing::debug;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Gpu {
    pub bdf: String,            // 0000:01:00.0
    pub vendor: String,         // 0x10de
    pub device: String,         // e.g. 0x1b80
    pub driver: Option<String>, // Some("nvidia") / Some("vfio-pci") / None
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum GpuState {
    None,
    Nvidia,
    Vfio,
    Other(String),
}

impl Gpu {
    pub fn state(&self) -> GpuState {
        match &self.driver {
            None => GpuState::None,
            Some(d) if d == "nvidia" => GpuState::Nvidia,
            Some(d) if d == "vfio-pci" => GpuState::Vfio,
            Some(d) => GpuState::Other(d.clone()),
        }
    }
}

pub fn list_nvidia_gpus() -> Result<Vec<Gpu>> {
    let mut out = vec![];
    for entry in fs::read_dir("/sys/bus/pci/devices").context("list pci devices")? {
        let path = entry?.path();
        let vendor = fs::read_to_string(path.join("vendor"))
            .unwrap_or_default()
            .trim()
            .to_string();
        if vendor.to_lowercase() != "0x10de" {
            continue;
        }
        let device = fs::read_to_string(path.join("device"))
            .unwrap_or_default()
            .trim()
            .to_string();
        let bdf = path.file_name().unwrap().to_string_lossy().into_owned();
        let driver = read_driver_name(&path.join("driver"));
        out.push(Gpu {
            bdf,
            vendor,
            device,
            driver,
        });
    }
    Ok(out)
}

fn read_driver_name(driver_link: &Path) -> Option<String> {
    if let Ok(meta) = fs::read_link(driver_link) {
        if let Some(name) = meta.file_name() {
            return Some(name.to_string_lossy().into_owned());
        }
    }
    None
}

fn ensure_module_loaded(name: &str) -> Result<()> {
    // если директория драйвера уже есть — всё ок
    let dir = format!("/sys/bus/pci/drivers/{}", name);
    if Path::new(&dir).exists() {
        return Ok(());
    }
    // иначе пробуем загрузить модуль
    // (демон под root, так что modprobe допустим)
    let st = std::process::Command::new("modprobe")
        .arg(name)
        .status()
        .context("spawn modprobe")?;
    if !st.success() {
        anyhow::bail!("modprobe {} failed with status {}", name, st);
    }
    Ok(())
}

fn write_str(path: &Path, val: &str) -> Result<()> {
    debug!("writing '{}' to {}", val, path.display());
    fs::write(path, val).with_context(|| format!("write to {}", path.display()))
}

fn ensure_module(modname: &str) -> Result<()> {
    // Lightweight: try writing to /sbin/modprobe via /proc
    // Fallback: /usr/bin/modprobe on PATH
    let status = std::process::Command::new("modprobe")
        .arg(modname)
        .status()?;
    if !status.success() {
        anyhow::bail!("modprobe {} failed with {:?}", modname, status);
    }
    Ok(())
}

pub fn unbind(bdf: &str) -> Result<()> {
    let dev = PathBuf::from("/sys/bus/pci/devices").join(bdf);
    let cur_driver = read_driver_name(&dev.join("driver"));
    if let Some(drv) = cur_driver {
        write_str(
            &PathBuf::from("/sys/bus/pci/drivers")
                .join(&drv)
                .join("unbind"),
            bdf,
        )?;
    }
    Ok(())
}

pub fn bind_to_nvidia(bdf: &str) -> Result<()> {
    ensure_module("nvidia")?;
    let path = PathBuf::from("/sys/bus/pci/drivers/nvidia/bind");
    write_str(&path, bdf)
}

fn dev_paths(bdf: &str) -> (PathBuf, PathBuf, PathBuf) {
    let dev = PathBuf::from(format!("/sys/bus/pci/devices/{}", bdf));
    (dev.clone(), dev.join("driver"), dev.join("driver_override"))
}

pub fn bind_to_vfio(bdf: &str) -> Result<()> {
    // Ensure the vfio-pci kernel module is loaded
    ensure_module_loaded("vfio-pci")?;

    let (_dev, driver_link, driver_override) = dev_paths(bdf);

    // 1) Set the device-level driver override to "vfio-pci"
    write_str(&driver_override, "vfio-pci\n").context("set driver_override=vfio-pci")?;

    // 2) If the device is already bound to another driver — unbind it
    if driver_link.exists() {
        let unbind = driver_link.join("unbind");
        write_str(&unbind, &format!("{}\n", bdf))
            .or_else(|e| {
                if e.downcast_ref::<std::io::Error>().map(|io| io.kind())
                    == Some(ErrorKind::ResourceBusy)
                {
                    // Device is busy — return a clear, user-friendly error
                    anyhow::bail!(
                        "device {} is busy; close users \
                        (Xorg/Wayland, CUDA, nvidia-persistenced) and retry",
                        bdf
                    );
                }
                Err(e)
            })
            .context("unbind from current driver")?;
    }

    // 3) Bind the device to the vfio-pci driver
    let vfio_bind = Path::new("/sys/bus/pci/drivers/vfio-pci/bind");
    write_str(vfio_bind, &format!("{}\n", bdf)).context("bind to vfio-pci")?;

    // 4) (Optional) Clear the override so that the device can be reattached normally later
    // If you prefer to keep the binding persistent across reboots or module reloads, skip this step.
    write_str(&driver_override, "\n").ok();

    Ok(())
}
