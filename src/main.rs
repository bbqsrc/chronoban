use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, Local};
use clap::Parser;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Parser, Debug)]
#[command(name = "chronoban")]
#[command(about = "Organize files into YYYY-MM directories based on modification time", long_about = None)]
struct Args {
    /// Directory to organize
    #[arg(default_value = ".")]
    path: PathBuf,

    /// Perform a dry run without moving files
    #[arg(short = 'n', long)]
    dry_run: bool,

    /// Only process files older than N days
    #[arg(short = 'a', long, default_value = "0")]
    min_age_days: u64,

    /// Recurse into subdirectories (does not recurse into YYYY-MM directories)
    #[arg(short, long)]
    recursive: bool,

    /// Use access time instead of modification time
    #[arg(long)]
    use_atime: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let path = args.path.canonicalize()
        .with_context(|| format!("Failed to access directory: {:?}", args.path))?;

    if !path.is_dir() {
        anyhow::bail!("Path must be a directory: {:?}", path);
    }

    println!("Organizing files in: {}", path.display());
    if args.dry_run {
        println!("🔍 DRY RUN MODE - No files will be moved");
    }
    println!();

    let stats = organize_directory(&path, &args)?;

    println!("\n📊 Summary:");
    println!("  Files moved: {}", stats.moved);
    println!("  Files skipped: {}", stats.skipped);
    println!("  Errors: {}", stats.errors);

    Ok(())
}

struct Stats {
    moved: usize,
    skipped: usize,
    errors: usize,
}

fn organize_directory(base_path: &Path, args: &Args) -> Result<Stats> {
    let mut stats = Stats {
        moved: 0,
        skipped: 0,
        errors: 0,
    };

    let min_age = std::time::Duration::from_secs(args.min_age_days * 24 * 60 * 60);
    let now = SystemTime::now();

    process_directory(base_path, base_path, args, &mut stats, min_age, now)?;

    Ok(stats)
}

fn process_directory(
    base_path: &Path,
    current_path: &Path,
    args: &Args,
    stats: &mut Stats,
    min_age: std::time::Duration,
    now: SystemTime,
) -> Result<()> {
    let entries = fs::read_dir(current_path)
        .with_context(|| format!("Failed to read directory: {:?}", current_path))?;

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                eprintln!("❌ Error reading entry: {}", e);
                stats.errors += 1;
                continue;
            }
        };

        let path = entry.path();
        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(e) => {
                eprintln!("❌ Error reading metadata for {:?}: {}", path, e);
                stats.errors += 1;
                continue;
            }
        };

        // Skip if it's already a YYYY-MM directory in the base path
        if path.is_dir() && is_year_month_dir(&path) && path.parent() == Some(base_path) {
            stats.skipped += 1;
            continue;
        }

        if metadata.is_dir() {
            if args.recursive && !is_year_month_dir(&path) {
                process_directory(base_path, &path, args, stats, min_age, now)?;
            }
            continue;
        }

        // Get the appropriate timestamp
        let file_time = if args.use_atime {
            metadata.accessed()
        } else {
            metadata.modified()
        };

        let file_time = match file_time {
            Ok(t) => t,
            Err(e) => {
                eprintln!("❌ Error reading timestamp for {:?}: {}", path, e);
                stats.errors += 1;
                continue;
            }
        };

        // Check minimum age
        if let Ok(age) = now.duration_since(file_time) {
            if age < min_age {
                stats.skipped += 1;
                continue;
            }
        }

        // Convert to DateTime
        let datetime: DateTime<Local> = file_time.into();
        let year_month = format!("{:04}-{:02}", datetime.year(), datetime.month());

        // Create target directory
        let target_dir = base_path.join(&year_month);
        let target_path = target_dir.join(path.file_name().unwrap());

        // Check if target already exists
        if target_path.exists() {
            eprintln!("⚠️  Target already exists, skipping: {} -> {}",
                path.display(), target_path.display());
            stats.skipped += 1;
            continue;
        }

        if args.dry_run {
            println!("📦 Would move: {} -> {}",
                path.display(), target_path.display());
            stats.moved += 1;
        } else {
            // Create the target directory if it doesn't exist
            if let Err(e) = fs::create_dir_all(&target_dir) {
                eprintln!("❌ Error creating directory {:?}: {}", target_dir, e);
                stats.errors += 1;
                continue;
            }

            // Move the file
            match fs::rename(&path, &target_path) {
                Ok(_) => {
                    println!("✅ Moved: {} -> {}",
                        path.display(), target_path.display());
                    stats.moved += 1;
                }
                Err(e) => {
                    eprintln!("❌ Error moving {:?}: {}", path, e);
                    stats.errors += 1;
                }
            }
        }
    }

    Ok(())
}

fn is_year_month_dir(path: &Path) -> bool {
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        // Check if it matches YYYY-MM pattern
        if name.len() == 7 && name.chars().nth(4) == Some('-') {
            let parts: Vec<&str> = name.split('-').collect();
            if parts.len() == 2 {
                return parts[0].parse::<u32>().is_ok() && parts[1].parse::<u32>().is_ok();
            }
        }
    }
    false
}
