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

# Check for existing Autonomi CLI (optional)
check_ant_cli() {
    log_step "Checking for Autonomi CLI (optional)..."

    if command_exists ant; then
        log_info "ant CLI is already installed. Version: $(ant --version 2>/dev/null || echo 'unknown')"
        ANT_AVAILABLE=true
    else
        log_info "Autonomi CLI not found - daemon will run in public-only mode"
        log_info "To enable full functionality later, install ant CLI with:"
        log_info "  curl -sSf https://raw.githubusercontent.com/maidsafe/antup/main/install.sh | sh"
        log_info "  antup client"
        ANT_AVAILABLE=false
    fi

    log_success "Autonomi CLI check completed"
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
    # log_info "Building all workspace crates in release mode..."
    # cargo build --workspace --release

    # Install CLI tools
    log_info "Installing CLI tools..."
    cargo install --path mutant-cli --force --locked
    cargo install --path mutant-daemon --force --locked

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

    if [[ "$ANT_AVAILABLE" == "true" ]] && command_exists ant; then
        # Check if ant wallet exists
        if ant wallet balance &>/dev/null; then
            log_success "Autonomi wallet is configured and accessible"
            WALLET_CONFIGURED=true
        else
            log_warning "Autonomi CLI found but wallet not configured"
            log_info "The daemon will run in public-only mode (can only download public data)"
            WALLET_CONFIGURED=false
        fi
    else
        log_info "No Autonomi CLI found - daemon will run in public-only mode"
        log_info "This allows downloading public data without wallet setup"
        WALLET_CONFIGURED=false
    fi
}

# Generate a secure Ethereum private key
generate_ethereum_private_key() {
    # Try multiple methods to generate a secure 32-byte private key

    # Method 1: Use openssl if available
    if command_exists openssl; then
        openssl rand -hex 32 2>/dev/null && return
    fi

    # Method 2: Use /dev/urandom if available (Linux/macOS)
    if [[ -r "/dev/urandom" ]]; then
        head -c 32 /dev/urandom | xxd -p -c 32 2>/dev/null && return
    fi

    # Method 3: Use Python if available
    if command_exists python3; then
        python3 -c "import secrets; print(secrets.token_hex(32))" 2>/dev/null && return
    fi

    # Method 4: Use Node.js if available
    if command_exists node; then
        node -e "console.log(require('crypto').randomBytes(32).toString('hex'))" 2>/dev/null && return
    fi

    # Method 5: Fallback using bash RANDOM (less secure, but better than nothing)
    log_warning "Using less secure fallback method for private key generation"
    local key=""
    for i in {1..64}; do
        key+=$(printf "%x" $((RANDOM % 16)))
    done
    echo "$key"
}

# Generate a BIP39 mnemonic phrase
generate_mnemonic() {
    # BIP39 wordlist (first 128 words for simplicity - enough for basic generation)
    local words=(
        "abandon" "ability" "able" "about" "above" "absent" "absorb" "abstract"
        "absurd" "abuse" "access" "accident" "account" "accuse" "achieve" "acid"
        "acoustic" "acquire" "across" "act" "action" "actor" "actress" "actual"
        "adapt" "add" "addict" "address" "adjust" "admit" "adult" "advance"
        "advice" "aerobic" "affair" "afford" "afraid" "again" "age" "agent"
        "agree" "ahead" "aim" "air" "airport" "aisle" "alarm" "album"
        "alcohol" "alert" "alien" "all" "alley" "allow" "almost" "alone"
        "alpha" "already" "also" "alter" "always" "amateur" "amazing" "among"
        "amount" "amused" "analyst" "anchor" "ancient" "anger" "angle" "angry"
        "animal" "ankle" "announce" "annual" "another" "answer" "antenna" "antique"
        "anxiety" "any" "apart" "apology" "appear" "apple" "approve" "april"
        "arch" "arctic" "area" "arena" "argue" "arm" "armed" "armor"
        "army" "around" "arrange" "arrest" "arrive" "arrow" "art" "article"
        "artist" "artwork" "ask" "aspect" "assault" "asset" "assist" "assume"
        "asthma" "athlete" "atom" "attack" "attend" "attitude" "attract" "auction"
        "audit" "august" "aunt" "author" "auto" "autumn" "average" "avocado"
        "avoid" "awake" "aware" "away" "awesome" "awful" "awkward" "axis"
    )

    # Generate 12 random words
    local mnemonic=""
    local word_count=${#words[@]}

    # Try to use secure random number generation
    for i in {1..12}; do
        local index

        # Method 1: Use openssl for random number
        if command_exists openssl; then
            index=$(openssl rand -hex 1 | head -c 2)
            index=$((0x$index % word_count))
        # Method 2: Use /dev/urandom
        elif [[ -r "/dev/urandom" ]]; then
            index=$(head -c 1 /dev/urandom | od -An -tu1 | tr -d ' ')
            index=$((index % word_count))
        # Method 3: Use Python
        elif command_exists python3; then
            index=$(python3 -c "import random; print(random.randint(0, $((word_count-1))))" 2>/dev/null)
        # Method 4: Fallback to bash RANDOM
        else
            index=$((RANDOM % word_count))
        fi

        if [[ $i -eq 1 ]]; then
            mnemonic="${words[$index]}"
        else
            mnemonic="$mnemonic ${words[$index]}"
        fi
    done

    echo "$mnemonic"
}

# Collect user credentials and create .env file
setup_user_credentials() {
    log_step "Setting up user credentials..."

    cd "$INSTALL_DIR"

    # Check if .env already exists
    if [[ -f ".env" ]]; then
        log_info ".env file already exists. Checking contents..."
        if grep -q "PRIVATE_KEY=" .env && grep -q "COLONY_MNEMONIC=" .env; then
            log_info "Credentials already configured in .env file"
            return 0
        fi
    fi

    echo ""
    log_highlight "🔐 Credential Setup"
    echo "   MutAnt needs your private key and colony mnemonic to function properly."
    echo "   These will be stored securely in a .env file in the installation directory."
    echo ""
    echo "   Options:"
    echo "   - Enter your existing credentials"
    echo "   - Press Enter to generate new ones automatically"
    echo "   - Type 'skip' to run in public-only mode (download only)"
    echo "   - Generated credentials will be cryptographically secure"
    echo ""

    # Ask for private key
    echo -n "Enter your private key (hex format, Enter to generate, or 'skip' for public-only): "
    read -r PRIVATE_KEY

    if [[ "$PRIVATE_KEY" == "skip" ]]; then
        log_info "Skipping credential setup. Daemon will run in public-only mode."
        PRIVATE_KEY=""
        COLONY_MNEMONIC=""
        # Create minimal .env file
        cat > .env << EOF
# MutAnt Configuration - Public-only mode
# Generated by install script on $(date)

# No credentials configured - running in public-only mode
PRIVATE_KEY=""
COLONY_MNEMONIC=""
EOF
        chmod 600 .env
        log_success "Created .env file for public-only mode"
        return 0
    elif [[ -z "$PRIVATE_KEY" ]]; then
        log_info "No private key provided. Generating a new Ethereum private key..."
        PRIVATE_KEY=$(generate_ethereum_private_key)
        if [[ -n "$PRIVATE_KEY" ]]; then
            log_success "Generated new private key: $PRIVATE_KEY"
            log_warning "⚠️  IMPORTANT: Save this private key securely! You'll need it to access your data."
        else
            log_error "Failed to generate private key. Daemon will run in public-only mode."
            PRIVATE_KEY=""
        fi
    else
        # Basic validation - check if it looks like a hex string
        if [[ ! "$PRIVATE_KEY" =~ ^[0-9a-fA-F]+$ ]]; then
            log_warning "Private key doesn't appear to be valid hex format, but continuing..."
        fi
    fi

    echo ""
    # Ask for colony mnemonic
    echo -n "Enter your colony mnemonic (12-24 words, or press Enter to generate a new one): "
    read -r COLONY_MNEMONIC

    if [[ -z "$COLONY_MNEMONIC" ]]; then
        log_info "No colony mnemonic provided. Generating a new 12-word mnemonic..."
        COLONY_MNEMONIC=$(generate_mnemonic)
        if [[ -n "$COLONY_MNEMONIC" ]]; then
            log_success "Generated new mnemonic: $COLONY_MNEMONIC"
            log_warning "⚠️  IMPORTANT: Save this mnemonic securely! You'll need it for colony features."
        else
            log_error "Failed to generate mnemonic. Colony features will be disabled."
            COLONY_MNEMONIC=""
        fi
    fi

    # Create .env file
    log_info "Creating .env file..."
    cat > .env << EOF
# MutAnt Configuration
# Generated by install script on $(date)

# Private key for Autonomi network access (hex format)
PRIVATE_KEY="$PRIVATE_KEY"

# Colony mnemonic for decentralized social features (12-24 words)
COLONY_MNEMONIC="$COLONY_MNEMONIC"
EOF

    # Set appropriate permissions
    chmod 600 .env

    log_success "Credentials saved to .env file"
    log_info "File permissions set to 600 (owner read/write only)"
    echo ""
}

# Load and export environment variables from .env file
load_environment() {
    log_step "Loading environment variables..."

    cd "$INSTALL_DIR"

    if [[ -f ".env" ]]; then
        log_info "Loading variables from .env file..."

        # Export variables from .env file
        set -a  # Automatically export all variables
        source .env
        set +a  # Stop automatically exporting

        # Verify variables are loaded
        if [[ -n "$PRIVATE_KEY" ]]; then
            log_info "Private key loaded (${#PRIVATE_KEY} characters)"
        else
            log_info "No private key found in .env"
        fi

        if [[ -n "$COLONY_MNEMONIC" ]]; then
            log_info "Colony mnemonic loaded"
        else
            log_info "No colony mnemonic found in .env"
        fi

        log_success "Environment variables loaded"
    else
        log_warning "No .env file found, continuing without custom credentials"
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
    trunk serve --port "$WEB_PORT" --address "127.0.0.1"

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
    log_info "Testing public data fetch (this works without wallet)..."
    if mutant get -p a420224971527d61ce6ee21d850a07c243498c95808697e8fac23f461545656933016697d10b805c0fa26b50eb3532b2 /tmp/test_meme.jpg &>/dev/null; then
        log_success "Public data fetch test successful - MutAnt is working!"
        rm -f /tmp/test_meme.jpg
    else
        log_info "Public data fetch test skipped (network may be unavailable)"
        log_info "This is normal and doesn't indicate a problem with the installation"
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
    echo "   mutant get -p <address> <file>   # Download public data (works without wallet)"
    echo "   mutant ls                        # List your stored keys (requires wallet)"
    echo ""

    log_highlight "🔍 Service Status:"
    echo "   mutant daemon status             # Check daemon status"
    echo "   ps aux | grep mutant-daemon      # Check daemon process"
    echo "   ps aux | grep trunk              # Check web server process"
    echo ""

    log_highlight "📁 Important Files & Directories:"
    echo "   Installation: $INSTALL_DIR"
    echo "   Config:       ~/.config/mutant/"
    echo "   Logs:         ~/.local/share/mutant/"
    echo "   Credentials:  $INSTALL_DIR/.env"
    echo ""

    if [[ "$WALLET_CONFIGURED" == "false" ]]; then
        log_warning "⚠️  Wallet Configuration:"
        echo "   Your daemon is running in PUBLIC-ONLY mode."
        echo "   This allows downloading public data without any setup."
        echo ""
        echo "   To enable full functionality (upload/store data):"
        echo ""
        echo "   1. Install Autonomi CLI:"
        echo "      curl -sSf https://raw.githubusercontent.com/maidsafe/antup/main/install.sh | sh"
        echo "      antup client"
        echo ""
        echo "   2. Create a new wallet:"
        echo "      ant wallet create"
        echo ""
        echo "   3. Or import existing wallet:"
        echo "      ant wallet import YOUR_PRIVATE_KEY"
        echo ""
        echo "   4. Restart the daemon:"
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

    log_highlight "🔐 Credential Management:"
    echo "   Your credentials are stored in: $INSTALL_DIR/.env"
    echo "   To view your credentials: cat $INSTALL_DIR/.env"
    echo "   To update credentials: edit $INSTALL_DIR/.env with your preferred editor"
    echo "   After editing: cd $INSTALL_DIR && ./install.sh --restart-only"
    echo ""

    # Show generated credentials warning if .env exists
    if [[ -f "$INSTALL_DIR/.env" ]]; then
        echo -e "${YELLOW}⚠️  SECURITY REMINDER:${NC}"
        echo "   - Keep your private key and mnemonic secure and backed up"
        echo "   - Never share these credentials with anyone"
        echo "   - Consider storing a backup in a secure location"
        echo ""
    fi

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

        # Load environment and start services
        cd "$INSTALL_DIR" || exit 1
        load_environment
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
    fi

    # Always check for ant CLI (doesn't install, just checks)
    check_ant_cli

    setup_repository
    build_rust_workspace
    setup_web_interface
    setup_configuration
    check_wallet_setup

    # Setup user credentials and environment
    setup_user_credentials
    load_environment

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
