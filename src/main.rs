mod arch;
mod civitai;
mod cli;
mod estimate;
mod hardware;
mod render;

fn main() {
    if let Err(error) = cli::run() {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}
