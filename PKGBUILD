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

  # Generate completions
  mkdir -p generate
  "./target/release/$pkgname" generate completions bash > "generate/$pkgname"
  "./target/release/$pkgname" generate completions zsh > "generate/_$pkgname"
  "./target/release/$pkgname" generate completions fish > "generate/$pkgname.fish"

  # Generate man page
  "./target/release/$pkgname" generate man > "generate/$pkgname.1"
}

package() {
  # cd "$srcdir/$pkgname-$pkgver"
  cd "$startdir"
  install -Dm755 "target/release/$pkgname" "$pkgdir/usr/bin/$pkgname"

  # Install completions
  install -Dm644 "generate/$pkgname" "$pkgdir/usr/share/bash-completion/completions/$pkgname"
  install -Dm644 "generate/_$pkgname" "$pkgdir/usr/share/zsh/site-functions/_$pkgname"
  install -Dm644 "generate/$pkgname.fish" "$pkgdir/usr/share/fish/vendor_completions.d/$pkgname.fish"

  # Install man page
  install -Dm644 "generate/$pkgname.1" "$pkgdir/usr/share/man/man1/$pkgname.1"
}
