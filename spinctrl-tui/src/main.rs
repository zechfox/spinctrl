mod app;
mod error;

use clap::Parser;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use env_logger;
use ratatui::{
    backend::CrosstermBackend,
    Terminal,
};
use std::io;
use crate::app::App;
use crate::error::Result;

#[derive(Parser)]
#[command(name = "spinctrl")]
#[command(about = "SpinCtrl system control tool for Acer Spin 13")]
#[command(version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,
    
    /// Run in check mode (verify service status and exit)
    #[arg(long)]
    check: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    
    // Initialize logging. Default is OFF: stderr output corrupts the TUI's
    // alternate-screen display. --verbose redirects to a file instead.
    if cli.verbose {
        match std::fs::OpenOptions::new().create(true).append(true).open("/tmp/spinctrl-tui.log") {
            Ok(f) => {
                env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug"))
                    .target(env_logger::Target::Pipe(Box::new(f)))
                    .init();
            }
            Err(_) => {
                env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug")).init();
            }
        }
    } else {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("off")).init();
    }
    
    log::info!("SpinCtrl TUI starting...");
    
    if cli.check {
        // Quick check mode
        return run_check_mode().await;
    }
    
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    
    // Create app and run it
    let mut app = App::new()?;
    let res = app.run(&mut terminal).await;
    
    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    
    if let Err(err) = res {
        eprintln!("Error: {}", err);
        std::process::exit(1);
    }
    
    Ok(())
}

async fn run_check_mode() -> Result<()> {
    let ipc = shared::IpcManager::new();
    
    match ipc.read_status() {
        Ok(Some(status)) => {
            println!("✓ Service is running (PID: {})", status.service_pid);
            println!("  Battery: {}%", status.battery.capacity);
            println!("  AC: {}", if status.power.ac_connected { "Connected" } else { "Disconnected" });
            println!("  CPU Governor: {}", status.power.cpu_governor);
            std::process::exit(0);
        }
        Ok(None) => {
            println!("✗ Service is not running");
            std::process::exit(1);
        }
        Err(e) => {
            println!("✗ Error checking service: {}", e);
            std::process::exit(1);
        }
    }
}