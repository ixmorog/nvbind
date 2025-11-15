use anyhow::Result;
use zbus::{Connection, zvariant::OwnedValue};

pub fn check_authorization(_action: &str) -> Result<()> {
    // Для brevity опустим вызов org.freedesktop.PolicyKit1.Authority.
    // На практике: вызываем CheckAuthorization для "org.example.nvbind.manage".
    Ok(())
}
