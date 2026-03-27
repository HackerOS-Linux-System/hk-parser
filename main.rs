use anyhow::{Context, Result};
use clap::Parser;
use hk_parser::{load_hk_file, parse_hk, resolve_interpolations, HkError};
use std::fs;
use std::path::PathBuf;

/// Hacker Lang configuration parser CLI
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Input .hk file
    #[arg(short, long)]
    input: PathBuf,

    /// Validate only (don't resolve interpolations)
    #[arg(short, long)]
    validate: bool,

    /// Resolve interpolations and output the result
    #[arg(short, long)]
    resolve: bool,

    /// Output file (if not provided, print to stdout)
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Pretty print errors with colors
    #[arg(long, default_value_t = true)]
    color: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Read the file
    let contents = fs::read_to_string(&args.input)
    .with_context(|| format!("Failed to read input file: {}", args.input.display()))?;

    // Parse
    let parse_result = parse_hk(&contents);
    match parse_result {
        Ok(mut config) => {
            if args.resolve {
                // Resolve interpolations
                if let Err(e) = resolve_interpolations(&mut config) {
                    e.pretty_print(&contents);
                    std::process::exit(1);
                }
            }

            // Output
            if let Some(output_path) = args.output {
                hk_parser::write_hk_file(output_path, &config)?;
            } else {
                // Print to stdout
                println!("{}", hk_parser::serialize_hk(&config));
            }
        }
        Err(e) => {
            e.pretty_print(&contents);
            std::process::exit(1);
        }
    }

    Ok(())
}
