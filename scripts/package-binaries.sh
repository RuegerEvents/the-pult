#!/usr/bin/env bash
#
# Archive the release binaries for one target, each with the licence, the readme
# and a checksum.
#
#     VERSION=0.0.1 TARGET=aarch64-apple-darwin scripts/package-binaries.sh
#
# Takes package names, or does the two that ship if given none. Writes into
# `archives/` beside the repository.
#
# The staging directory is built up by hand rather than by pointing a tool at
# cargo's output, which is where this started: that directory also holds cargo's
# dep-info file, and a tool that archives all of it puts `pult-backend.d` in
# beside the binary — and, having made itself a second copy of the binary under
# the release name, both of those too. An archive should hold one copy of one
# program.
set -euo pipefail

version="${VERSION:?set VERSION}"
target="${TARGET:?set TARGET}"

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

packages=("$@")
if [ ${#packages[@]} -eq 0 ]; then
	packages=(pult-backend openhaunt-sim)
fi

# Linux has one of these and macOS the other; Git Bash on Windows has the first.
checksum() {
	if command -v sha256sum > /dev/null 2>&1; then
		sha256sum "$1"
	else
		shasum -a 256 "$1"
	fi
}

mkdir -p archives

for package in "${packages[@]}"; do
	echo "==> $package $version $target"
	cargo build --release --locked --target "$target" --package "$package"

	exe=""
	if [ -f "target/$target/release/$package.exe" ]; then
		exe=".exe"
	fi
	binary="target/$target/release/$package$exe"
	if [ ! -f "$binary" ]; then
		echo "no binary at $binary" >&2
		exit 1
	fi

	name="$package-$version-$target"
	stage="$(mktemp -d)/$name"
	mkdir -p "$stage"
	cp "$binary" "$stage/"
	cp LICENSE README.md "$stage/"

	if [ -n "$exe" ]; then
		archive="$name.zip"
		# PowerShell rather than tar: whether the `tar` on PATH in Git Bash is the
		# one that can write a zip depends on which one is found first, and this
		# one is on every Windows runner. Run from inside the staging directory so
		# there are no Unix paths for it to fail to understand.
		( cd "$stage" && powershell -NoProfile -Command \
			"Compress-Archive -Path '$package$exe','LICENSE','README.md' -DestinationPath 'archive.zip' -Force" )
		mv "$stage/archive.zip" "$root/archives/$archive"
	else
		archive="$name.tar.gz"
		tar -C "$stage" -czf "$root/archives/$archive" "$package$exe" LICENSE README.md
	fi

	if [ ! -f "$root/archives/$archive" ]; then
		echo "no archive at archives/$archive" >&2
		exit 1
	fi
	( cd "$root/archives" && checksum "$archive" > "$archive.sha256" )
	rm -rf "$(dirname "$stage")"
done

ls -l archives
