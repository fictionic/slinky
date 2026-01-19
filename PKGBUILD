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
source=("$pkgname-$pkgver.tar.gz::https://github.com/yourusername/slinky/archive/v$pkgver.tar.gz")
# Note: Since this is local, you can use 'makepkg -e' or point to your local dir

build() {
  cd "$srcdir/$pkgname-$pkgver"
  cargo build --release --locked
}

package() {
  cd "$srcdir/$pkgname-$pkgver"
  install -Dm755 "target/release/$pkgname" "$pkgdir/usr/bin/$pkgname"
}
