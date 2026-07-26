#!/usr/bin/env bash
# Install khora from source.
#
#   ./install.sh                     # -> ~/.cargo/bin/khora
#   ./install.sh --prefix /usr/local # -> /usr/local/bin/khora
set -euo pipefail

root=""
while [ $# -gt 0 ]; do
	case "$1" in
	--prefix)
		root="${2:?--prefix needs a directory}"
		shift 2
		;;
	--prefix=*)
		root="${1#*=}"
		shift
		;;
	-h | --help)
		sed -n '2,5p' "$0" | cut -c3-
		exit 0
		;;
	*)
		echo "unknown option: $1" >&2
		exit 1
		;;
	esac
done

if ! command -v cargo >/dev/null 2>&1; then
	echo "cargo not found — install Rust from https://rustup.rs" >&2
	exit 1
fi

cd "$(dirname "$0")"

if [ -n "$root" ]; then
	cargo install --path crates/khora-cli --locked --force --root "$root"
	bin="$root/bin"
else
	cargo install --path crates/khora-cli --locked --force
	bin="${CARGO_INSTALL_ROOT:-${CARGO_HOME:-$HOME/.cargo}}/bin"
fi

echo
"$bin/khora" --version
case ":$PATH:" in
*":$bin:"*) ;;
*) echo "note: $bin is not on your PATH" >&2 ;;
esac
