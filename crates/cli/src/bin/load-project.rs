use std::path::PathBuf;

use clap::Parser;

#[derive(Parser)]
struct Cli {
    project_file: PathBuf,
}

fn main() {
    let cli = Cli::parse();
    let proj = fabricator::Project::load(&cli.project_file).unwrap();
    println!("{:#?}", &proj);
}
