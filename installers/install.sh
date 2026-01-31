#!/bin/sh
# isq installer for macOS and Linux
# Usage: curl -LsSf https://cameronwestland.com/isq/install.sh | sh

set -eu

REPO="camwest/isq"

# Colors (if terminal supports them)
if [ -t 1 ]; then
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    YELLOW='\033[1;33m'
    CYAN='\033[0;36m'
    NC='\033[0m'
else
    RED='' GREEN='' YELLOW='' CYAN='' NC=''
fi

error() { printf "${RED}error${NC}: %s\n" "$1" >&2; exit 1; }
info() { printf "${CYAN}=>${NC} %s\n" "$1" >&2; }
success() { printf "${GREEN}ok${NC} %s\n" "$1" >&2; }

detect_platform() {
    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Darwin) os_target="apple-darwin" ;;
        Linux) os_target="unknown-linux-gnu" ;;
        *) error "Unsupported OS: $os. Only macOS and Linux are supported." ;;
    esac

    case "$arch" in
        x86_64 | amd64) arch_target="x86_64" ;;
        arm64 | aarch64) arch_target="aarch64" ;;
        *) error "Unsupported architecture: $arch. Only x86_64 and arm64 are supported." ;;
    esac

    echo "${arch_target}-${os_target}"
}

check_prerequisites() {
    if command -v curl >/dev/null 2>&1; then
        DOWNLOAD_CMD="curl -fsSL -o"
    elif command -v wget >/dev/null 2>&1; then
        DOWNLOAD_CMD="wget -q -O"
    else
        error "Neither curl nor wget found. Please install one of them first."
    fi

    command -v tar >/dev/null 2>&1 || error "tar not found. Please install tar first."

    if command -v shasum >/dev/null 2>&1; then
        SHA_CMD="shasum -a 256"
    elif command -v sha256sum >/dev/null 2>&1; then
        SHA_CMD="sha256sum"
    else
        error "Neither shasum nor sha256sum found. Cannot verify download integrity."
    fi
}

download() {
    $DOWNLOAD_CMD "$2" "$1" || return 1
}

get_latest_version() {
    url="https://api.github.com/repos/${REPO}/releases/latest"
    tmpfile=$(mktemp)

    download "$url" "$tmpfile" || error "Failed to fetch release info from GitHub API"
    response=$(cat "$tmpfile")
    rm -f "$tmpfile"

    case "$response" in
        *"API rate limit exceeded"*)
            error "GitHub API rate limit exceeded. Try again later or download from: https://github.com/${REPO}/releases/latest"
            ;;
    esac

    version=$(echo "$response" | grep '"tag_name"' | head -1 | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/')
    [ -z "$version" ] && error "No releases found. Check https://github.com/${REPO}/releases"

    echo "$version"
}

download_and_verify() {
    version="$1"
    target="$2"
    archive="isq-${target}.tar.gz"
    base_url="https://github.com/${REPO}/releases/download/${version}"

    tmpdir=$(mktemp -d) || error "Failed to create temp directory"

    info "Downloading ${archive}..."
    download "${base_url}/${archive}" "${tmpdir}/${archive}" || \
        error "Failed to download ${archive}. Check your network connection."

    info "Downloading checksums..."
    download "${base_url}/checksums.txt" "${tmpdir}/checksums.txt" || \
        error "Failed to download checksums.txt"

    info "Verifying checksum..."
    expected_hash=$(grep "${archive}" "${tmpdir}/checksums.txt" | head -1 | awk '{print $1}')
    [ -z "$expected_hash" ] && { rm -rf "$tmpdir"; error "Checksum for ${archive} not found"; }

    actual_hash=$(cd "$tmpdir" && $SHA_CMD "$archive" | awk '{print $1}')
    if [ "$expected_hash" != "$actual_hash" ]; then
        rm -rf "$tmpdir"
        error "Checksum verification failed! Expected: $expected_hash, Got: $actual_hash"
    fi

    success "Checksum verified"
    echo "$tmpdir"
}

determine_install_dir() {
    default_dir="/usr/local/bin"

    if [ -w "$default_dir" ]; then
        echo "$default_dir"
    elif command -v sudo >/dev/null 2>&1 && { sudo -n true 2>/dev/null || [ -t 0 ]; }; then
        echo "sudo:$default_dir"
    else
        fallback_dir="$HOME/.local/bin"
        mkdir -p "$fallback_dir" || error "Failed to create $fallback_dir"
        echo "$fallback_dir"
    fi
}

install_binary() {
    tmpdir="$1"
    install_spec="$2"

    case "$install_spec" in
        sudo:*) use_sudo=true; install_dir="${install_spec#sudo:}" ;;
        *) use_sudo=false; install_dir="$install_spec" ;;
    esac

    info "Extracting..."
    tar -xzf "${tmpdir}"/isq-*.tar.gz -C "$tmpdir" || error "Failed to extract archive"

    info "Installing to ${install_dir}..."
    if [ "$use_sudo" = true ]; then
        sudo install -m 755 "${tmpdir}/isq" "${install_dir}/isq" || error "Failed to install binary"
    else
        install -m 755 "${tmpdir}/isq" "${install_dir}/isq" || error "Failed to install binary"
    fi

    rm -rf "$tmpdir"
    echo "${install_dir}/isq"
}

verify_binary() {
    if ! "$1" --version >/dev/null 2>&1; then
        error "Installation completed but binary failed to run. Check library dependencies."
    fi
}

write_receipt() {
    if ! "$1" install write-receipt --method standalone --binary-path "$1" --auto-update 2>/dev/null; then
        printf "${YELLOW}warning${NC}: Failed to write install receipt\n" >&2
    fi
}

check_path() {
    install_dir="$1"
    case ":$PATH:" in
        *":$install_dir:"*) return 0 ;;
    esac

    printf "${YELLOW}warning${NC}: %s is not in your PATH.\n\n" "$install_dir" >&2
    printf "Add it to your shell profile:\n\n"
    printf "  # For bash\n"
    printf "  echo 'export PATH=\"\$PATH:%s\"' >> ~/.bashrc\n\n" "$install_dir"
    printf "  # For zsh\n"
    printf "  echo 'export PATH=\"\$PATH:%s\"' >> ~/.zshrc\n\n" "$install_dir"
    printf "Then restart your shell or run: export PATH=\"\$PATH:%s\"\n" "$install_dir"
}

main() {
    info "Installing isq..."
    check_prerequisites

    target=$(detect_platform)
    info "Detected platform: $target"

    version=$(get_latest_version)
    info "Latest version: $version"

    tmpdir=$(download_and_verify "$version" "$target")
    install_spec=$(determine_install_dir)
    binary_path=$(install_binary "$tmpdir" "$install_spec")
    verify_binary "$binary_path"
    write_receipt "$binary_path"

    install_dir="${install_spec#sudo:}"
    [ "$install_dir" = "$HOME/.local/bin" ] && check_path "$install_dir"

    echo ""
    success "isq ${version} installed to ${binary_path}"
    echo ""
    echo "Run 'isq --help' to get started."
}

main "$@"
