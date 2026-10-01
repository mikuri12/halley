mod auth;
mod compositor;
mod pipewire;
mod screencast;
mod screenshot;
mod session;

use zbus::blocking::Connection;

const BUS_NAME: &str = "org.freedesktop.impl.portal.desktop.halley";
const OBJECT_PATH: &str = "/org/freedesktop/portal/desktop";

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args
        .iter()
        .any(|arg| matches!(arg.as_str(), "-h" | "--help"))
    {
        print_help();
        return;
    }
    if args
        .iter()
        .any(|arg| matches!(arg.as_str(), "-V" | "--version"))
    {
        println!("xdg-desktop-portal-halley {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if !args.is_empty() {
        eprintln!("xdg-desktop-portal-halley: unknown argument {:?}", args[0]);
        std::process::exit(2);
    }

    if let Err(err) = pollster::block_on(eventline::setup(eventline::Setup {
        verbose: true,
        level: Some(eventline::LogLevel::Info),
        console_level: None,
        file_level: None,
        file: None,
        journal_retention: None,
    })) {
        eprintln!("portal logging setup failed: {err}");
    }
    if let Err(err) = run() {
        eventline::error!("portal failed: {err}");
        std::process::exit(1);
    }
}

fn print_help() {
    println!("xdg-desktop-portal-halley {}", env!("CARGO_PKG_VERSION"));
    println!();
    println!("Halley ScreenCast and Screenshot portal backend.");
    println!("Normally started by xdg-desktop-portal through D-Bus activation.");
    println!();
    println!("Options:");
    println!("  -h, --help     Show this help");
    println!("  -V, --version  Show the version");
}

fn run() -> zbus::Result<()> {
    let connection = Connection::session()?;
    let producer = std::sync::Arc::new(pipewire::Producer::new());
    connection.object_server().at(
        OBJECT_PATH,
        screencast::ScreenCastInterface::new(connection.clone(), producer),
    )?;
    connection.object_server().at(
        OBJECT_PATH,
        screenshot::ScreenshotInterface::new(connection.clone()),
    )?;
    connection.request_name(BUS_NAME)?;
    eventline::info!("portal ready: bus={BUS_NAME} object={OBJECT_PATH}");

    loop {
        std::thread::park();
    }
}
