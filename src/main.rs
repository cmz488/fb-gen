use clap::Parser;

fn main() {
    let cli = fb_gen::cli::Cli::parse();
    fb_gen::cli::run(cli);
}
