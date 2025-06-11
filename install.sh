#!/bin/bash

# MutAnt All-in-One Installation and Setup Script
# Download and run with: curl -sSf https://raw.githubusercontent.com/Champii/Anthill/master/install.sh | bash

set -e  # Exit on any error

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
PURPLE='\033[0;35m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# Configuration
REPO_URL="https://github.com/Champii/Anthill.git"
REPO_BRANCH="master"
INSTALL_DIR="$HOME/mutant"
DAEMON_PORT="3030"
WEB_PORT="8080"

# Logging functions
log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

log_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

log_step() {
    echo -e "${PURPLE}[STEP]${NC} $1"
}

log_highlight() {
    echo -e "${CYAN}[HIGHLIGHT]${NC} $1"
}

# Print banner
print_banner() {
    echo ""
    echo -e "${CYAN}╔══════════════════════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${CYAN}║                                                                              ║${NC}"
    echo -e "${CYAN}║                        ${GREEN}MutAnt All-in-One Installer${CYAN}                        ║${NC}"
    echo -e "${CYAN}║                                                                              ║${NC}"
    echo -e "${CYAN}║           ${YELLOW}Decentralized P2P Mutable Key-Value Storage for Autonomi${CYAN}           ║${NC}"
    echo -e "${CYAN}║                                                                              ║${NC}"
    echo -e "${CYAN}╚══════════════════════════════════════════════════════════════════════════════╝${NC}"
    echo ""
}

# Detect OS and package manager
detect_os() {
    log_step "Detecting operating system..."
    
    if [[ "$OSTYPE" == "linux-gnu"* ]]; then
        OS="linux"
        if command -v apt-get &> /dev/null; then
            PACKAGE_MANAGER="apt"
        elif command -v yum &> /dev/null; then
            PACKAGE_MANAGER="yum"
        elif command -v pacman &> /dev/null; then
            PACKAGE_MANAGER="pacman"
        elif command -v zypper &> /dev/null; then
            PACKAGE_MANAGER="zypper"
        else
            log_error "Unsupported Linux distribution. Please install dependencies manually."
            exit 1
        fi
    elif [[ "$OSTYPE" == "darwin"* ]]; then
        OS="macos"
        PACKAGE_MANAGER="brew"
    elif [[ "$OSTYPE" == "msys" ]] || [[ "$OSTYPE" == "cygwin" ]]; then
        OS="windows"
        PACKAGE_MANAGER="choco"
    else
        log_error "Unsupported operating system: $OSTYPE"
        exit 1
    fi
    
    log_info "Detected OS: $OS with package manager: $PACKAGE_MANAGER"
}

# Check if command exists
command_exists() {
    command -v "$1" &> /dev/null
}

# Install system dependencies
install_system_deps() {
    log_step "Installing system dependencies..."
    
    case $PACKAGE_MANAGER in
        "apt")
            sudo apt-get update -y
            sudo apt-get install -y \
                curl \
                git \
                build-essential \
                pkg-config \
                libssl-dev \
                procps \
                wget \
                unzip \
                python3 \
                python3-pip \
                ffmpeg \
                libavformat-dev \
                libavcodec-dev \
                libavutil-dev \
                libavfilter-dev \
                libavdevice-dev \
                libswscale-dev \
                libswresample-dev \
                clang \
                llvm-dev \
                libclang-dev
            ;;
        "yum")
            sudo yum update -y
            sudo yum groupinstall -y "Development Tools"
            sudo yum install -y \
                curl \
                git \
                openssl-devel \
                procps-ng \
                wget \
                unzip \
                python3 \
                python3-pip \
                ffmpeg \
                ffmpeg-devel \
                clang \
                llvm-devel
            ;;
        "pacman")
            sudo pacman -Syu --noconfirm
            sudo pacman -S --noconfirm \
                curl \
                git \
                base-devel \
                openssl \
                procps-ng \
                wget \
                unzip \
                python \
                python-pip \
                ffmpeg \
                clang \
                llvm
            ;;
        "zypper")
            sudo zypper refresh
            sudo zypper install -y \
                curl \
                git \
                gcc \
                make \
                openssl-devel \
                procps \
                wget \
                unzip \
                python3 \
                python3-pip \
                ffmpeg \
                ffmpeg-devel \
                clang \
                llvm-devel
            ;;
        "brew")
            if ! command_exists brew; then
                log_info "Installing Homebrew..."
                /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
            fi
            brew update
            brew install \
                curl \
                git \
                openssl \
                wget \
                python3 \
                ffmpeg \
                llvm
            ;;
        "choco")
            if ! command_exists choco; then
                log_error "Chocolatey not found. Please install Chocolatey first: https://chocolatey.org/install"
                exit 1
            fi
            choco install -y \
                curl \
                git \
                wget \
                unzip \
                python3 \
                ffmpeg \
                llvm
            ;;
    esac
    
    log_success "System dependencies installed"
}

# Verify FFmpeg installation
verify_ffmpeg() {
    log_step "Verifying FFmpeg installation..."

    if ! command_exists ffmpeg; then
        log_error "FFmpeg not found in PATH. Video transcoding will not work."
        return 1
    fi

    if ! command_exists ffprobe; then
        log_error "FFprobe not found in PATH. Video metadata extraction will not work."
        return 1
    fi

    # Test FFmpeg functionality
    log_info "Testing FFmpeg functionality..."
    if ffmpeg -version &>/dev/null; then
        log_success "FFmpeg is working correctly"
        log_info "FFmpeg version: $(ffmpeg -version 2>/dev/null | head -n1)"
    else
        log_warning "FFmpeg may not be working correctly"
        return 1
    fi

    return 0
}

# Set up environment variables for FFmpeg compilation
setup_ffmpeg_env() {
    log_step "Setting up FFmpeg environment variables..."

    # Set environment variables for ffmpeg-sys-next compilation
    case $OS in
        "linux")
            # For Ubuntu/Debian systems
            if [[ -d "/usr/include/libavformat" ]]; then
                export FFMPEG_INCLUDE_DIR="/usr/include"
                export FFMPEG_LIB_DIR="/usr/lib/x86_64-linux-gnu"
            fi
            # For other Linux systems
            if [[ -d "/usr/local/include/libavformat" ]]; then
                export FFMPEG_INCLUDE_DIR="/usr/local/include"
                export FFMPEG_LIB_DIR="/usr/local/lib"
            fi
            ;;
        "macos")
            # For Homebrew on macOS
            if [[ -d "/opt/homebrew/include/libavformat" ]]; then
                export FFMPEG_INCLUDE_DIR="/opt/homebrew/include"
                export FFMPEG_LIB_DIR="/opt/homebrew/lib"
            elif [[ -d "/usr/local/include/libavformat" ]]; then
                export FFMPEG_INCLUDE_DIR="/usr/local/include"
                export FFMPEG_LIB_DIR="/usr/local/lib"
            fi
            ;;
    esac

    # Set PKG_CONFIG_PATH for better library detection
    if command_exists pkg-config; then
        export PKG_CONFIG_PATH="$PKG_CONFIG_PATH:/usr/local/lib/pkgconfig:/opt/homebrew/lib/pkgconfig"
    fi

    log_success "FFmpeg environment configured"
}

# Install Rust toolchain
install_rust() {
    log_step "Installing Rust toolchain..."
    
    if command_exists rustc; then
        log_info "Rust is already installed. Version: $(rustc --version)"
        log_info "Updating Rust toolchain..."
        rustup update
    else
        log_info "Installing Rust via rustup..."
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        source "$HOME/.cargo/env"
    fi
    
    # Ensure we have the latest stable toolchain
    log_info "Installing Rust stable toolchain..."
    rustup install stable
    rustup default stable
    
    # Install required components
    log_info "Installing Rust components..."
    rustup component add clippy
    rustup component add rustfmt
    
    # Add wasm target for web builds
    log_info "Adding WebAssembly target..."
    rustup target add wasm32-unknown-unknown
    
    # Install wasm-pack for web builds
    log_info "Installing wasm-pack..."
    if ! command_exists wasm-pack; then
        curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh
    else
        log_info "wasm-pack is already installed"
    fi
    
    # Install trunk for web serving
    log_info "Installing trunk..."
    if ! command_exists trunk; then
        cargo install trunk
    else
        log_info "trunk is already installed"
    fi
    
    log_success "Rust toolchain installed and configured"
}

# Install Node.js and npm
install_nodejs() {
    log_step "Installing Node.js..."
    
    if command_exists node; then
        log_info "Node.js is already installed. Version: $(node --version)"
    else
        case $PACKAGE_MANAGER in
            "apt")
                curl -fsSL https://deb.nodesource.com/setup_lts.x | sudo -E bash -
                sudo apt-get install -y nodejs
                ;;
            "yum")
                curl -fsSL https://rpm.nodesource.com/setup_lts.x | sudo bash -
                sudo yum install -y nodejs npm
                ;;
            "pacman")
                sudo pacman -S --noconfirm nodejs npm
                ;;
            "zypper")
                sudo zypper install -y nodejs npm
                ;;
            "brew")
                brew install node
                ;;
            "choco")
                choco install -y nodejs
                ;;
        esac
    fi
    
    # Install pnpm if not present
    if ! command_exists pnpm; then
        log_info "Installing pnpm..."
        npm install -g pnpm
    fi
    
    log_success "Node.js and package managers installed"
}

# Install Autonomi CLI (ant)
install_ant_cli() {
    log_step "Installing Autonomi CLI (ant)..."

    if command_exists ant; then
        log_info "ant CLI is already installed. Version: $(ant --version 2>/dev/null || echo 'unknown')"
    else
        log_info "Installing ant CLI via antup..."
        curl -sSf https://raw.githubusercontent.com/maidsafe/antup/main/install.sh | sh

        # Add to PATH if not already there
        if [[ ":$PATH:" != *":$HOME/.local/bin:"* ]]; then
            echo 'export PATH="$HOME/.local/bin:$PATH"' >> "$HOME/.bashrc"
            export PATH="$HOME/.local/bin:$PATH"
        fi

        # Install the client
        if command_exists antup; then
            antup client
        else
            log_warning "antup installation may have failed. Will continue in public-only mode."
        fi
    fi

    log_success "Autonomi CLI installation completed"
}

# Clone or update repository
setup_repository() {
    log_step "Setting up MutAnt repository..."

    if [[ -d "$INSTALL_DIR" ]]; then
        log_info "Repository directory exists. Updating..."
        cd "$INSTALL_DIR"
        git fetch origin
        git reset --hard origin/$REPO_BRANCH
        git clean -fd
    else
        log_info "Cloning repository..."
        git clone -b "$REPO_BRANCH" "$REPO_URL" "$INSTALL_DIR"
        cd "$INSTALL_DIR"
    fi

    log_success "Repository setup completed"
}

# Build Rust workspace
build_rust_workspace() {
    log_step "Building Rust workspace..."

    cd "$INSTALL_DIR"

    # Check if we're in the right directory
    if [[ ! -f "Cargo.toml" ]]; then
        log_error "Cargo.toml not found. Repository may be corrupted."
        exit 1
    fi

    # Build all workspace members in release mode
    log_info "Building all workspace crates in release mode..."
    cargo build --workspace --release

    # Install CLI tools
    log_info "Installing CLI tools..."
    cargo install --path mutant-cli --force
    cargo install --path mutant-daemon --force

    log_success "Rust workspace built and installed successfully"
}

# Setup web dependencies and build
setup_web_interface() {
    log_step "Setting up web interface..."

    cd "$INSTALL_DIR"

    if [[ -d "mutant-web" ]]; then
        cd mutant-web

        # Install Node.js dependencies
        if [[ -f "package.json" ]]; then
            log_info "Installing Node.js dependencies with pnpm..."
            pnpm install
        fi

        # Build WASM module with trunk
        log_info "Building web interface with trunk..."
        trunk build --release

        cd ..
        log_success "Web interface setup completed"
    else
        log_warning "mutant-web directory not found, skipping web setup"
    fi
}

# Setup configuration directories
setup_configuration() {
    log_step "Setting up configuration..."

    # Create necessary directories
    mkdir -p ~/.config/mutant
    mkdir -p ~/.local/share/mutant

    log_success "Configuration directories created"
}

# Check for wallet setup
check_wallet_setup() {
    log_step "Checking wallet setup..."

    if command_exists ant; then
        # Check if ant wallet exists
        if ant wallet balance &>/dev/null; then
            log_success "Autonomi wallet is configured and accessible"
            WALLET_CONFIGURED=true
        else
            log_warning "Autonomi wallet not configured or not accessible"
            log_info "The daemon will run in public-only mode (can only download public data)"
            WALLET_CONFIGURED=false
        fi
    else
        log_warning "Autonomi CLI not available"
        log_info "The daemon will run in public-only mode (can only download public data)"
        WALLET_CONFIGURED=false
    fi
}

# Start daemon
start_daemon() {
    log_step "Starting MutAnt daemon..."

    # Check if daemon is already running
    if pgrep -f "mutant-daemon" > /dev/null; then
        log_info "MutAnt daemon is already running"
        return 0
    fi

    # Start daemon in background
    log_info "Starting daemon on port $DAEMON_PORT..."
    nohup mutant-daemon --bind "127.0.0.1:$DAEMON_PORT" > ~/.local/share/mutant/daemon.log 2>&1 &

    # Wait a moment for daemon to start
    sleep 3

    # Check if daemon started successfully
    if pgrep -f "mutant-daemon" > /dev/null; then
        log_success "MutAnt daemon started successfully"
        log_info "Daemon logs: ~/.local/share/mutant/daemon.log"
    else
        log_error "Failed to start MutAnt daemon"
        log_info "Check logs at: ~/.local/share/mutant/daemon.log"
        return 1
    fi
}

# Start web server
start_web_server() {
    log_step "Starting web server..."

    cd "$INSTALL_DIR/mutant-web"

    # Check if web server is already running
    if pgrep -f "trunk serve" > /dev/null; then
        log_info "Web server is already running"
        return 0
    fi

    # Start web server in background
    log_info "Starting web server on port $WEB_PORT..."
    nohup trunk serve --port "$WEB_PORT" --address "127.0.0.1" > ~/.local/share/mutant/web.log 2>&1 &

    # Wait a moment for web server to start
    sleep 3

    # Check if web server started successfully
    if pgrep -f "trunk serve" > /dev/null; then
        log_success "Web server started successfully"
        log_info "Web server logs: ~/.local/share/mutant/web.log"
    else
        log_error "Failed to start web server"
        log_info "Check logs at: ~/.local/share/mutant/web.log"
        return 1
    fi
}

# Test basic functionality
test_functionality() {
    log_step "Testing basic functionality..."

    # Test daemon connection
    log_info "Testing daemon connection..."
    sleep 2

    # Try to connect to daemon via CLI
    if mutant daemon status &>/dev/null; then
        log_success "Daemon is responding to CLI commands"
    else
        log_warning "Daemon may not be fully ready yet"
    fi

    # Test public data fetch (this should work even without wallet)
    log_info "Testing public data fetch..."
    if mutant get -p a420224971527d61ce6ee21d850a07c243498c95808697e8fac23f461545656933016697d10b805c0fa26b50eb3532b2 /tmp/test_meme.jpg &>/dev/null; then
        log_success "Public data fetch test successful"
        rm -f /tmp/test_meme.jpg
    else
        log_warning "Public data fetch test failed (this may be normal if network is unavailable)"
    fi
}

# Print final instructions
print_final_instructions() {
    echo ""
    echo -e "${CYAN}╔══════════════════════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${CYAN}║                                                                              ║${NC}"
    echo -e "${CYAN}║                        ${GREEN}Installation Complete!${CYAN}                             ║${NC}"
    echo -e "${CYAN}║                                                                              ║${NC}"
    echo -e "${CYAN}╚══════════════════════════════════════════════════════════════════════════════╝${NC}"
    echo ""

    log_success "MutAnt has been successfully installed and started!"
    echo ""

    log_highlight "🌐 Web Interface:"
    echo "   Open your browser and go to: http://127.0.0.1:$WEB_PORT"
    echo ""

    log_highlight "🎥 Video Support:"
    echo "   FFmpeg is installed for video transcoding and streaming"
    echo "   Supports: MP4, WebM, AVI, MKV, MOV, FLV, WMV, and more"
    echo ""

    log_highlight "🔧 Command Line Interface:"
    echo "   mutant --help                    # Show CLI help"
    echo "   mutant ls                        # List your stored keys"
    echo "   mutant get -p <address> <file>   # Download public data"
    echo ""

    log_highlight "🔍 Service Status:"
    echo "   mutant daemon status             # Check daemon status"
    echo "   ps aux | grep mutant-daemon      # Check daemon process"
    echo "   ps aux | grep trunk              # Check web server process"
    echo ""

    log_highlight "📁 Important Directories:"
    echo "   Installation: $INSTALL_DIR"
    echo "   Config:       ~/.config/mutant/"
    echo "   Logs:         ~/.local/share/mutant/"
    echo ""

    if [[ "$WALLET_CONFIGURED" == "false" ]]; then
        log_warning "⚠️  Wallet Configuration:"
        echo "   Your daemon is running in PUBLIC-ONLY mode."
        echo "   To enable full functionality (upload/store data):"
        echo ""
        echo "   1. Create a new wallet:"
        echo "      ant wallet create"
        echo ""
        echo "   2. Or import existing wallet:"
        echo "      ant wallet import YOUR_PRIVATE_KEY"
        echo ""
        echo "   3. Restart the daemon:"
        echo "      pkill mutant-daemon"
        echo "      mutant daemon start"
        echo ""
    else
        log_success "✅ Wallet is configured - full functionality available!"
    fi

    log_highlight "🛑 To Stop Services:"
    echo "   pkill mutant-daemon              # Stop daemon"
    echo "   pkill trunk                      # Stop web server"
    echo ""

    log_highlight "🔄 To Restart Services:"
    echo "   cd $INSTALL_DIR && ./install.sh --restart-only"
    echo ""

    log_info "For more information, visit: https://github.com/Champii/Anthill"
    echo ""
}

# Cleanup function
cleanup() {
    log_info "Cleaning up temporary files..."
    # Add any cleanup tasks here if needed
}

# Signal handlers
trap cleanup EXIT

# Parse command line arguments
parse_arguments() {
    RESTART_ONLY=false
    SKIP_DEPS=false

    while [[ $# -gt 0 ]]; do
        case $1 in
            --restart-only)
                RESTART_ONLY=true
                shift
                ;;
            --skip-deps)
                SKIP_DEPS=true
                shift
                ;;
            --help|-h)
                echo "MutAnt All-in-One Installer"
                echo ""
                echo "Usage: $0 [OPTIONS]"
                echo ""
                echo "Options:"
                echo "  --restart-only    Only restart services (skip installation)"
                echo "  --skip-deps       Skip dependency installation"
                echo "  --help, -h        Show this help message"
                echo ""
                exit 0
                ;;
            *)
                log_error "Unknown option: $1"
                echo "Use --help for usage information"
                exit 1
                ;;
        esac
    done
}

# Main installation function
main() {
    # Parse arguments first
    parse_arguments "$@"

    print_banner

    if [[ "$RESTART_ONLY" == "true" ]]; then
        log_step "Restarting services only..."

        # Stop existing services
        pkill mutant-daemon || true
        pkill trunk || true
        sleep 2

        # Start services
        cd "$INSTALL_DIR" || exit 1
        start_daemon
        start_web_server

        log_success "Services restarted!"
        echo ""
        log_highlight "🌐 Web Interface: http://127.0.0.1:$WEB_PORT"
        exit 0
    fi

    # Full installation
    detect_os

    if [[ "$SKIP_DEPS" != "true" ]]; then
        install_system_deps
        verify_ffmpeg
        setup_ffmpeg_env
        install_rust
        install_nodejs
        install_ant_cli
    fi

    setup_repository
    build_rust_workspace
    setup_web_interface
    setup_configuration
    check_wallet_setup

    # Start services
    start_daemon
    start_web_server

    # Test functionality
    test_functionality

    # Print final instructions
    print_final_instructions
}

# Run main function with all arguments
main "$@"
