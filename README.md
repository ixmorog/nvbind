# nvbind

nvbind is a tiny toolkit for switching NVIDIA GPUs between the proprietary `nvidia` kernel
driver and `vfio-pci`. It is useful on systems where a discrete NVIDIA GPU is sometimes
dedicated to a virtual machine but should otherwise remain attached to the host driver.

The repository contains three crates:

| Component     | Description |
| ------------- | ----------- |
| `nvbind-core` | Shared library that discovers NVIDIA PCI devices and performs bind/unbind operations by writing to sysfs. |
| `nvbindd`     | System D-Bus service that exposes the core operations via the `org.example.NvBind` interface. Authorization is delegated to PolicyKit so only trusted users can rebind devices. |
| `nvbind-tray` | Optional desktop tray application that talks to `nvbindd`, displays GPU status, and offers menu actions to refresh or switch drivers. |

## Building

```bash
cargo build --workspace --release
```

`nvbindd` is intended to run as a system service. Install the binary and ship a service
unit that executes `nvbindd` as root. The tray application can be installed into the user
profile and autostarted via XDG Autostart.

## Running the daemon

```bash
# enable nvbindd as a system service
systemctl enable --now nvbindd
```

Once the daemon is online, clients can connect to the well-known name `org.example.NvBind`
at object path `/org/example/NvBind` and invoke the following methods:

| Method            | Purpose |
| ----------------- | ------- |
| `GetStatus()`     | Returns a JSON string that contains all NVIDIA GPUs and their current drivers. |
| `BindToNvidia(s)` | Unbinds the given BDF and reattaches it to the appropriate host driver (display functions go to `nvidia`, audio/USB functions are resolved via the PCI modalias). |
| `BindToVfio(s)`   | Same as above but binds to `vfio-pci`. |
| `Unbind(s)`       | Only performs the unbind step, leaving the device driverless. |

All mutating methods require the caller to be authorized for the `org.example.nvbind.manage`
PolicyKit action.

## Tray application

`nvbind-tray` runs as a regular user process. It embeds a small Tokio runtime that polls
the daemon every few seconds and listens for menu commands. Notifications are shown via
`notify-rust` whenever a command succeeds or fails.

## Contributing

Pull requests are welcome. Please ensure the code remains rustfmt clean and that comments,
documentation, and user-visible strings are written in English so the project remains
friendly to contributors worldwide.
