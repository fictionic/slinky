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

  for bin in "$pkgname" "$pkgname-ln"; do
    # Binary
    install -Dm755 "target/release/$bin" "$pkgdir/usr/bin/$bin"
    # Install completions
    install -Dm644 "generate/$bin.bash" "$pkgdir/usr/share/bash-completion/completions/$bin"
    install -Dm644 "generate/_$bin" "$pkgdir/usr/share/zsh/site-functions/_$bin"
    install -Dm644 "generate/$bin.fish" "$pkgdir/usr/share/fish/vendor_completions.d/$bin.fish"
    install -Dm644 "generate/$bin.1" "$pkgdir/usr/share/man/man1/$bin.1"
  done
}
