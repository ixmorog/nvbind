use anyhow::{Result, anyhow};
use serde::Serialize;
use std::collections::HashMap;
use zbus::zvariant::{OwnedValue, Type};
use zbus::{Connection, Proxy, names::BusName};

/// Polkit subject is a DBus STRUCT: (s, a{sv})
/// Use positional fields + `Serialize` to encode as a struct, NOT a dict.
#[derive(Type, Serialize)]
struct Subject<'a>(&'a str, HashMap<String, OwnedValue>);

/// Check that the sender is authorized for org.example.nvbind.manage.
/// Flags include AllowUserInteraction (1), so an auth prompt can be shown.
pub async fn check_authz(conn: &Connection, sender_unique: &str) -> Result<()> {
    // 1) Resolve PID/UID of the sender via org.freedesktop.DBus
    let dbus = zbus::fdo::DBusProxy::new(conn).await?;
    let bus_name = BusName::try_from(sender_unique)
        .map_err(|e| anyhow!("invalid unique bus name '{}': {}", sender_unique, e))?;
    let pid = dbus.get_connection_unix_process_id(bus_name).await?;

    // BusName was moved above; recreate it for the next call
    let bus_name = BusName::try_from(sender_unique)?;
    let uid = dbus.get_connection_unix_user(bus_name).await?;

    // 2) Build subject (unix-process, a{sv})
    let mut sv: HashMap<String, OwnedValue> = HashMap::new();
    sv.insert("pid".into(), OwnedValue::from(pid));
    sv.insert("uid".into(), OwnedValue::from(uid));
    // start-time can be 0 if unknown
    sv.insert("start-time".into(), OwnedValue::from(0u64));
    let subject = Subject("unix-process", sv);

    // 3) Authority proxy
    let auth = Proxy::new(
        conn,
        "org.freedesktop.PolicyKit1",
        "/org/freedesktop/PolicyKit1/Authority",
        "org.freedesktop.PolicyKit1.Authority",
    )
    .await?;

    // 4) Prepare args: (subject, action_id, details, flags, cancellation_id)
    let action_id = "org.example.nvbind.manage".to_string();
    let details: HashMap<String, String> = HashMap::new(); // a{ss}
    let flags: u32 = 1; // AllowUserInteraction
    let cancel_id = ""; // s

    // 5) Call and decode the tuple return: (b authorized, b challenge, a{ss} details)
    let (authorized, _challenge, _ret_details): (bool, bool, HashMap<String, String>) = auth
        .call(
            "CheckAuthorization",
            &(subject, action_id, details, flags, cancel_id),
        )
        .await?;

    if authorized {
        Ok(())
    } else {
        Err(anyhow!("Polkit: not authorized"))
    }
}
