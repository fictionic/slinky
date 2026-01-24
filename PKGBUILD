# Maintainer: Dylan Forbes <dylandforbes@gmail.com>
pkgname=slinky
pkgver=0.1.0
pkgrel=1
pkgdesc="Manage symlinks"
arch=('x86_64')
url="https://github.com/yourusername/slinky"
license=('MIT')
depends=('gcc-libs')
makedepends=('rust')
# source=("$pkgname-$pkgver.tar.gz::https://github.com/fictionic/slinky/archive/v$pkgver.tar.gz")
source=()
options=('!debug')

build() {
  # cd "$srcdir/$pkgname-$pkgver"
  cd "$startdir"
  cargo build --release --locked
}

package() {
  # cd "$srcdir/$pkgname-$pkgver"
  cd "$startdir"
  install -Dm755 "target/release/$pkgname" "$pkgdir/usr/bin/$pkgname"
}
