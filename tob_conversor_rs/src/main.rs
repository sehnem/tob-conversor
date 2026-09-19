#!/usr/bin/env rust-script

use clap::{Parser, ValueEnum};
use rayon::prelude::*;
use std::path::{Path, PathBuf};

#[derive(ValueEnum, Clone, Debug, Default)]
enum OutputFormat {
    #[default]
    Toa5,
    Parquet,
}

#[derive(Parser, Debug)]
#[command(
    name = "tobconversor",
    about = "Convert Campbell TOB1/TOB2/TOB3 binary files to TOA5 or Parquet"
)]
struct Args {
    /// Directory containing .dat files (searched recursively)
    input: PathBuf,

    /// Output directory (default: sibling folder `<input_stem>_converted`)
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Number of parallel worker threads (0 = all available CPUs)
    #[arg(short, long, default_value_t = 0)]
    jobs: usize,

    /// Include the RECORD column (logger record number) in the output
    #[arg(long)]
    record: bool,

    /// Output format
    #[arg(long, value_enum, default_value_t = OutputFormat::Toa5)]
    format: OutputFormat,
}

struct Task {
    file: PathBuf,
    out_dir: PathBuf,
}

/// Collect all `.dat` files under `dir`, recursively.
fn collect_dat_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                files.extend(collect_dat_files(&path));
            } else if path.extension().map(|e| e == "dat").unwrap_or(false) {
                files.push(path);
            }
        }
    }
    files
}

fn process_folder(
    input: &Path,
    output: &Path,
    jobs: usize,
    include_record: bool,
    format: &OutputFormat,
) -> usize {
    let files = collect_dat_files(input);
    if files.is_empty() {
        return 0;
    }

    let mut tasks: Vec<Task> = files
        .into_iter()
        .map(|file| {
            let stem = file.file_stem().unwrap_or_default().to_string_lossy();
            let out_dir = output.join(stem.as_ref());
            Task { file, out_dir }
        })
        .collect();

    tasks.sort_by(|a, b| a.file.cmp(&b.file));
    run_tasks(tasks, jobs, include_record, format)
}

const SPLIT_INTERVAL_MINUTES: usize = 30;

fn convert_one(task: &Task, include_record: bool, format: &OutputFormat) -> Result<usize, String> {
    match format {
        OutputFormat::Toa5 => tobconversor::convert_streaming(
            &task.file,
            &task.out_dir,
            SPLIT_INTERVAL_MINUTES,
            include_record,
        ),
        OutputFormat::Parquet => {
            tobconversor::convert_tob_to_parquet(&task.file, &task.out_dir, include_record)
        }
    }
}

fn run_tasks(tasks: Vec<Task>, jobs: usize, include_record: bool, format: &OutputFormat) -> usize {
    let n_tasks = tasks.len();
    if n_tasks == 0 {
        return 0;
    }

    println!("\nTotal: {} .dat file(s) to convert", n_tasks);
    let thread_display = if jobs == 0 {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    } else {
        jobs
    };
    if jobs != 1 {
        println!("Parallel: {} worker threads", thread_display);
    }

    if jobs == 1 {
        let mut acc = 0;
        for task in tasks {
            let name = task.file.file_name().unwrap_or_default().to_string_lossy();
            println!("  Converting {}...", name);
            match convert_one(&task, include_record, format) {
                Ok(n) => acc += n,
                Err(e) => eprintln!("  Error converting {}: {}", name, e),
            }
        }
        acc
    } else {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(jobs)
            .build()
            .unwrap();
        let results: Vec<usize> = pool.install(|| {
            tasks
                .into_par_iter()
                .map(|task| {
                    let name = task
                        .file
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    match convert_one(&task, include_record, format) {
                        Ok(n) => {
                            println!("  OK {} → {} output file(s)", name, n);
                            n
                        }
                        Err(e) => {
                            let trunc_err: String = e.chars().take(200).collect();
                            let trunc_err = if e.chars().count() > 200 {
                                format!("{}...", trunc_err)
                            } else {
                                trunc_err
                            };
                            eprintln!("  ERROR {}: {}", name, trunc_err);
                            0
                        }
                    }
                })
                .collect()
        });
        results.iter().sum()
    }
}

fn main() {
    let args = Args::parse();

    let output = args.output.unwrap_or_else(|| {
        let dirname = args.input.file_name().unwrap_or_default().to_string_lossy();
        args.input
            .parent()
            .unwrap_or(&args.input)
            .join(format!("{}_converted", dirname))
    });

    println!("{:=<60}", "");
    println!("TOB1/TOB2/TOB3 converter");
    println!("{:=<60}", "");
    println!("Input:   {}", args.input.display());
    println!("Output:  {}", output.display());
    let display_jobs = if args.jobs == 0 {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    } else {
        args.jobs
    };
    println!("Jobs:    {} ({} parallel threads)", args.jobs, display_jobs);
    println!("Format:  {:?}", args.format);
    println!(
        "RECORD:  {}",
        if args.record {
            "yes (--record)"
        } else {
            "no (default)"
        }
    );

    let total_written = process_folder(&args.input, &output, args.jobs, args.record, &args.format);

    println!("\n{:=<60}", "");
    println!("Done: {} output file(s) written.", total_written);
    println!("{:=<60}", "");
}
