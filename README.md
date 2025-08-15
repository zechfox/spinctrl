# SpinCtrl

A Rust and Bash utility for controlling Acer Spin 13 Chromebook features.

## Overview

SpinCtrl provides system-level control and automation for Acer Spin 13 Chromebook hardware features, including display rotation, keyboard/touchpad management, and other device-specific functionality.

## Features

- Hardware control interface for Acer Spin 13
- System automation scripts
- Cross-platform compatibility where applicable

## Requirements

- Rust (latest stable)
- Bash shell
- Linux-based system (ChromeOS/Linux)

## Installation

```bash
git clone https://github.com/yourusername/spinctrl.git
cd spinctrl
cargo build --release
```

## Usage

```bash
# Build the project
cargo build

# Run the main binary
cargo run

# Install system-wide
cargo install --path .
```

## Project Structure

```
spinctrl/
├── src/           # Rust source code
├── scripts/       # Bash scripts
├── Cargo.toml     # Rust project configuration
└── README.md      # This file
```

## Contributing

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Submit a pull request

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Compatibility

Designed specifically for:
- Acer Spin 13 Chromebook
- ChromeOS and Linux environments