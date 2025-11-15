# Maintainer: MX <ixmorog@gmail.com>
pkgname=nvbind
pkgver=0.1.0
pkgrel=1
pkgdesc="NVIDIA GPU binding manager with system tray and D-Bus daemon"
arch=('x86_64')
url="https://github.com/ixmorog/nvbind"
license=('MIT')
depends=('dbus' 'polkit')
makedepends=('cargo' 'clang' 'pkgconf')
source=("$pkgname-$pkgver.tar.gz")
sha256sums=('SKIP')

build() {
  cd "$srcdir/$pkgname-$pkgver"
  export RUSTFLAGS="-C target-cpu=native"
  cargo build --release --locked
}

package() {
  cd "$srcdir/$pkgname-$pkgver"
  install -Dm755 "target/release/nvbindd" "$pkgdir/usr/bin/nvbindd"
  install -Dm755 "target/release/nvbind-tray" "$pkgdir/usr/bin/nvbind-tray"

  # systemd
  install -Dm644 packaging/systemd/nvbindd.service "$pkgdir/usr/lib/systemd/system/nvbindd.service"

  # dbus (system)
  install -Dm644 packaging/dbus/org.example.NvBind.conf "$pkgdir/usr/share/dbus-1/system.d/org.example.NvBind.conf"
  install -Dm644 packaging/dbus/org.example.NvBind.service "$pkgdir/usr/share/dbus-1/system-services/org.example.NvBind.service"

  # polkit
  install -Dm644 packaging/polkit/org.example.nvbind.policy "$pkgdir/usr/share/polkit-1/actions/org.example.nvbind.policy"
  install -Dm644 packaging/polkit/50-nvbind.rules "$pkgdir/usr/share/polkit-1/rules.d/50-nvbind.rules"

  # autostart
  install -Dm644 packaging/xdg/nvbind-tray.desktop "$pkgdir/etc/xdg/autostart/nvbind-tray.desktop"

  # docs
  install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
  install -Dm644 README.md "$pkgdir/usr/share/doc/$pkgname/README.md"
}
