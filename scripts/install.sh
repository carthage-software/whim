#!/usr/bin/env bash

set -euo pipefail

REPOSITORY="carthage-software/whim"
BINARY="whim"
NEW_ISSUE="https://github.com/carthage-software/whim/issues/new"
SIGNER_WORKFLOW=".github/workflows/release.yml"
INSTALL_DIRECTORY=""
VERSION=""
VERIFY_MODE="auto"
VERIFY_ATTESTATION=0
TEMPORARY_DIRECTORY=$(mktemp -d)

separator() {
  printf '\n\033[39m======================================================================\033[0m\n\n'
}

red() {
  printf '\033[31m%s\033[0m\n' "$1"
}

green() {
  printf '\033[32m%s\033[0m\n' "$1"
}

yellow() {
  printf '\033[33m%s\033[0m\n' "$1"
}

blue() {
  printf '\033[34m%s\033[0m\n' "$1"
}

fail() {
  red "$1"
  exit 1
}

cleanup() {
  rm -rf "$TEMPORARY_DIRECTORY"
}

trap cleanup EXIT

download() {
  local url="$1"
  local destination="$2"
  local asset="$3"

  if command -v curl > /dev/null; then
    if ! curl -fL "$url" -o "$destination"; then
      fail "Failed to download ${asset}."
    fi

    return
  fi

  if command -v wget > /dev/null; then
    if ! wget -q --show-progress "$url" -O "$destination"; then
      fail "Failed to download ${asset}."
    fi

    return
  fi

  fail "Neither 'curl' nor 'wget' is installed."
}

gh_supports_attestation() {
  command -v gh > /dev/null && gh attestation --help > /dev/null 2>&1
}

blue "Welcome to the Whim installer!"
blue "This script will download and install Whim for your system."
printf '\n'
yellow "If you encounter a problem, open an issue at ${NEW_ISSUE}."

for argument in "$@"; do
  case "$argument" in
    --install-dir=*)
      INSTALL_DIRECTORY="${argument#*=}"
      ;;
    --version=*)
      VERSION="${argument#*=}"
      ;;
    --always-verify)
      if [ "$VERIFY_MODE" = "never" ]; then
        fail "Cannot combine --always-verify with --no-verify."
      fi

      VERIFY_MODE="always"
      ;;
    --no-verify)
      if [ "$VERIFY_MODE" = "always" ]; then
        fail "Cannot combine --always-verify with --no-verify."
      fi

      VERIFY_MODE="never"
      ;;
    *)
      fail "Unknown argument: ${argument}"
      ;;
  esac
done

separator

green "Detecting your system configuration..."
architecture=$(uname -m)
operating_system=$(uname -s | tr '[:upper:]' '[:lower:]')

case "$architecture" in
  x86_64 | amd64)
    architecture="x86_64"
    ;;
  arm64 | aarch64)
    architecture="aarch64"
    ;;
  riscv64)
    architecture="riscv64gc"
    ;;
  *)
    fail "Unsupported architecture: ${architecture}."
    ;;
esac

case "$operating_system" in
  darwin)
    case "$architecture" in
      x86_64 | aarch64)
        target="${architecture}-apple-darwin"
        ;;
      *)
        fail "Whim does not provide a macOS build for ${architecture}."
        ;;
    esac
    ;;
  linux)
    case "$architecture" in
      x86_64 | aarch64 | riscv64gc)
        ;;
      *)
        fail "Whim does not provide a Linux build for ${architecture}."
        ;;
    esac

    glibc=0
    if command -v getconf > /dev/null && getconf GNU_LIBC_VERSION > /dev/null 2>&1; then
      glibc=1
    elif command -v ldd > /dev/null && ldd --version 2>&1 | grep -Eiq 'glibc|gnu libc'; then
      glibc=1
    fi

    if [ "$glibc" -ne 1 ]; then
      fail "Whim release builds require a glibc-based Linux system."
    fi

    target="${architecture}-unknown-linux-gnu"
    ;;
  *)
    fail "Unsupported operating system: ${operating_system}."
    ;;
esac

green "Detected target: ${target}"

separator

binary_directory=""
if [ -n "$INSTALL_DIRECTORY" ]; then
  binary_directory="$INSTALL_DIRECTORY"
  if [ ! -d "$binary_directory" ]; then
    fail "The installation directory does not exist: ${binary_directory}"
  fi

  if [ ! -w "$binary_directory" ]; then
    fail "The installation directory is not writable: ${binary_directory}"
  fi
else
  possible_directories=("/usr/local/bin" "/usr/bin" "/bin")
  for directory in "${possible_directories[@]}"; do
    if [ ! -d "$directory" ]; then
      yellow "The directory ${directory} does not exist. Trying the next directory..."
      continue
    fi

    if [ ! -w "$directory" ]; then
      yellow "The directory ${directory} is not writable. Trying the next directory..."
      continue
    fi

    binary_directory="$directory"
    break
  done

  if [ -z "$binary_directory" ]; then
    yellow "No writable system binary directory was found. Using the current directory."
    binary_directory=$(pwd)
  fi
fi

green "Whim will be installed to: ${binary_directory}"

separator

if [ -n "$VERSION" ]; then
  release_tag="$VERSION"
  green "Installing release: ${release_tag}"
else
  green "Fetching the latest Whim release..."
  if command -v curl > /dev/null; then
    response=$(curl -fsSL "https://api.github.com/repos/${REPOSITORY}/releases/latest") || {
      fail "Failed to fetch the latest release."
    }
  elif command -v wget > /dev/null; then
    response=$(wget -q -O - "https://api.github.com/repos/${REPOSITORY}/releases/latest") || {
      fail "Failed to fetch the latest release."
    }
  else
    fail "Neither 'curl' nor 'wget' is installed."
  fi

  release_tag=$(printf '%s\n' "$response" | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1)
  if [ -z "$release_tag" ]; then
    fail "The latest release response did not contain a tag."
  fi

  green "Latest release: ${release_tag}"
fi

if [[ ! "$release_tag" =~ ^v?[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  fail "Invalid release tag: ${release_tag}"
fi

version="${release_tag#v}"
package="${BINARY}-${version}-${target}"
archive="${package}.tar.gz"
release_url="https://github.com/${REPOSITORY}/releases/download/${release_tag}"
archive_path="${TEMPORARY_DIRECTORY}/${archive}"

if [ "$version" = "0.1.0" ]; then
  if [ "$VERIFY_MODE" = "always" ]; then
    fail "Whim 0.1.0 has no build attestation and cannot be verified."
  fi

  separator
  yellow "Attestation verification is unavailable for Whim 0.1.0."
else
  case "$VERIFY_MODE" in
    always)
      separator
      if ! command -v gh > /dev/null; then
        red "--always-verify requires the GitHub CLI ('gh'), but it was not found in PATH."
        red "Install it from https://cli.github.com/ and run this script again."
        fail "Refusing to install without attestation verification."
      fi

      if ! gh attestation --help > /dev/null 2>&1; then
        red "--always-verify requires a GitHub CLI version with 'gh attestation'."
        red "Upgrade the GitHub CLI from https://cli.github.com/ and run this script again."
        fail "Refusing to install without attestation verification."
      fi

      VERIFY_ATTESTATION=1
      green "Attestation verification: ON (--always-verify). Using: $(command -v gh)"
      ;;
    never)
      separator
      yellow "Attestation verification: OFF (--no-verify)."
      yellow "The downloaded archive will be installed without checking its build attestation."
      ;;
    auto)
      separator
      if gh_supports_attestation; then
        VERIFY_ATTESTATION=1
        green "Attestation verification: ON (auto). Using: $(command -v gh)"
      else
        yellow "Attestation verification: OFF."
        if command -v gh > /dev/null; then
          yellow "The installed GitHub CLI does not support 'gh attestation'."
          yellow "Upgrade it from https://cli.github.com/ to enable verification."
        else
          yellow "Install the GitHub CLI from https://cli.github.com/ to enable verification."
        fi
        yellow "Pass --always-verify to require verification."
      fi
      ;;
  esac
fi

separator

green "Downloading ${archive}..."
download "${release_url}/${archive}" "$archive_path" "$archive"
green "Download complete!"

if [ "$VERIFY_ATTESTATION" -eq 1 ]; then
  separator
  green "Verifying the build attestation for ${archive}..."
  green "  Repository:      ${REPOSITORY}"
  green "  Signer workflow: ${SIGNER_WORKFLOW}"
  if ! gh attestation verify "$archive_path" \
    --repo "$REPOSITORY" \
    --signer-workflow "${REPOSITORY}/${SIGNER_WORKFLOW}"; then
    quarantine="${PWD}/${archive%.tar.gz}.unverified.tar.gz"
    cp "$archive_path" "$quarantine" || quarantine=""

    red "Attestation verification failed for ${archive}."
    red "The archive did not match an attestation signed by ${REPOSITORY}/${SIGNER_WORKFLOW}."
    red "Refusing to install it."
    if [ -n "$quarantine" ]; then
      red "The unverified archive was preserved at:"
      red "  ${quarantine}"
    fi

    fail "Installation aborted."
  fi

  green "Attestation verified!"
fi

separator

green "Extracting ${archive}..."
if ! tar -xzf "$archive_path" -C "$TEMPORARY_DIRECTORY"; then
  fail "Failed to extract ${archive}."
fi
green "Extraction complete!"

separator

green "Installing Whim to ${binary_directory}..."
if ! mv "${TEMPORARY_DIRECTORY}/${package}/${BINARY}" "${binary_directory}/${BINARY}"; then
  fail "Failed to install Whim to ${binary_directory}."
fi

if ! chmod +x "${binary_directory}/${BINARY}"; then
  fail "Failed to make ${binary_directory}/${BINARY} executable."
fi

green "Installation complete!"

if ! printf '%s' "$PATH" | tr ':' '\n' | grep -Fxq "$binary_directory"; then
  printf '\n'
  yellow "The directory ${binary_directory} is not in your PATH."
  yellow "Add it for the current shell with:"
  printf '  export PATH=%q:\$PATH\n' "$binary_directory"
fi
