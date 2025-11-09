use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, Local};
use clap::Parser;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tokio::fs;

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

    /// Only process items older than N days
    #[arg(short = 'a', long, default_value = "0")]
    min_age_days: u64,

    /// Use access time instead of modification time
    #[arg(long)]
    use_atime: bool,

    /// Maximum number of concurrent move operations
    #[arg(short = 'j', long, default_value = "16")]
    jobs: usize,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let path = tokio::fs::canonicalize(&args.path)
        .await
        .with_context(|| format!("Failed to access directory: {:?}", args.path))?;

    if !path.is_dir() {
        anyhow::bail!("Path must be a directory: {:?}", path);
    }

    println!("Organizing files in: {}", path.display());
    if args.dry_run {
        println!("🔍 DRY RUN MODE - No files will be moved");
    }
    println!();

    let stats = organize_directory(&path, &args).await?;

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

async fn organize_directory(base_path: &Path, args: &Args) -> Result<Stats> {
    let mut stats = Stats {
        moved: 0,
        skipped: 0,
        errors: 0,
    };

    let min_age = std::time::Duration::from_secs(args.min_age_days * 24 * 60 * 60);
    let now = SystemTime::now();

    // Read directory entries
    let mut entries = fs::read_dir(base_path)
        .await
        .with_context(|| format!("Failed to read directory: {:?}", base_path))?;

    let mut tasks = Vec::new();

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();

        // Skip YYYY-MM directories
        if is_year_month_dir(&path) {
            stats.skipped += 1;
            continue;
        }

        let metadata = match entry.metadata().await {
            Ok(m) => m,
            Err(e) => {
                eprintln!("❌ Error reading metadata for {:?}: {}", path, e);
                stats.errors += 1;
                continue;
            }
        };

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

        // Create target directory path
        let target_dir = base_path.join(&year_month);
        let target_path = target_dir.join(path.file_name().unwrap());

        // Check if target already exists
        if target_path.exists() {
            eprintln!("⚠️  Target already exists, skipping: {} -> {}",
                path.display(), target_path.display());
            stats.skipped += 1;
            continue;
        }

        let dry_run = args.dry_run;

        // Spawn async task for moving
        let task = tokio::spawn(async move {
            if dry_run {
                println!("📦 Would move: {} -> {}", path.display(), target_path.display());
                Ok::<_, anyhow::Error>(true)
            } else {
                // Create target directory
                fs::create_dir_all(&target_dir).await
                    .with_context(|| format!("Failed to create directory: {:?}", target_dir))?;

                // Move the file/directory
                fs::rename(&path, &target_path).await
                    .with_context(|| format!("Failed to move {:?} to {:?}", path, target_path))?;

                println!("✅ Moved: {} -> {}", path.display(), target_path.display());
                Ok(true)
            }
        });

        tasks.push(task);

        // Limit concurrent tasks
        if tasks.len() >= args.jobs {
            let task = tasks.remove(0);
            match task.await {
                Ok(Ok(_)) => stats.moved += 1,
                Ok(Err(e)) => {
                    eprintln!("❌ Error: {}", e);
                    stats.errors += 1;
                }
                Err(e) => {
                    eprintln!("❌ Task error: {}", e);
                    stats.errors += 1;
                }
            }
        }
    }

    // Wait for remaining tasks
    for task in tasks {
        match task.await {
            Ok(Ok(_)) => stats.moved += 1,
            Ok(Err(e)) => {
                eprintln!("❌ Error: {}", e);
                stats.errors += 1;
            }
            Err(e) => {
                eprintln!("❌ Task error: {}", e);
                stats.errors += 1;
            }
        }
    }

    Ok(stats)
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
