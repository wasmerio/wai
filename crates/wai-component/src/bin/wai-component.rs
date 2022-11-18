use clap::Parser;
use wai_component::cli::WaiComponentApp;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_target(false)
        .init();

    if let Err(e) = WaiComponentApp::parse().execute() {
        log::error!("{:?}", e);
        std::process::exit(1);
    }
}
