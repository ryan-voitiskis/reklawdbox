use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::tags;

use super::{display_field_name, expand_paths};

#[derive(clap::Args)]
pub(crate) struct ReadTagsArgs {
    /// Audio files or directories to read
    #[arg(required = true)]
    paths: Vec<String>,
    /// Only return these fields (comma-separated)
    #[arg(long, value_delimiter = ',')]
    fields: Option<Vec<String>>,
    /// Include cover art metadata
    #[arg(long)]
    cover_art: bool,
    /// Recursively scan directories for audio files
    #[arg(long, short = 'r')]
    recursive: bool,
    /// Output as JSON (JSONL for multiple files)
    #[arg(long)]
    json: bool,
}

#[derive(clap::Args)]
pub(crate) struct WriteTagsArgs {
    /// Audio file to write tags to
    #[arg(required = true)]
    path: String,
    #[arg(long)]
    artist: Option<String>,
    #[arg(long)]
    title: Option<String>,
    #[arg(long)]
    album: Option<String>,
    #[arg(long)]
    album_artist: Option<String>,
    #[arg(long)]
    genre: Option<String>,
    #[arg(long)]
    year: Option<String>,
    #[arg(long)]
    track: Option<String>,
    #[arg(long)]
    disc: Option<String>,
    #[arg(long)]
    comment: Option<String>,
    #[arg(long)]
    publisher: Option<String>,
    #[arg(long)]
    bpm: Option<String>,
    #[arg(long)]
    key: Option<String>,
    #[arg(long)]
    composer: Option<String>,
    #[arg(long)]
    remixer: Option<String>,
    /// Preview changes without writing
    #[arg(long)]
    dry_run: bool,
    /// WAV tag targets: id3v2, riff_info (comma-separated, default: both)
    #[arg(long, value_delimiter = ',')]
    wav_targets: Option<Vec<String>>,
    /// Read tags from stdin as JSON (overrides individual field flags)
    #[arg(long)]
    json_input: bool,
    /// Output as JSON
    #[arg(long)]
    json: bool,
}

#[derive(clap::Args)]
pub(crate) struct ExtractArtArgs {
    /// Audio file to extract art from
    #[arg(required = true)]
    path: String,
    /// Output path (default: cover.{ext} in same directory)
    #[arg(long)]
    output: Option<String>,
    /// Picture type (default: front_cover)
    #[arg(long, default_value = "front_cover")]
    picture_type: String,
    /// Output as JSON
    #[arg(long)]
    json: bool,
}

#[derive(clap::Args)]
pub(crate) struct EmbedArtArgs {
    /// Image file to embed
    #[arg(required = true)]
    image: String,
    /// Audio files to embed art into
    #[arg(required = true)]
    targets: Vec<String>,
    /// Picture type (default: front_cover)
    #[arg(long, default_value = "front_cover")]
    picture_type: String,
    /// Output as JSON
    #[arg(long)]
    json: bool,
}

fn print_tags_human(result: &tags::FileReadResult) {
    match result {
        tags::FileReadResult::Single {
            path,
            format,
            tag_type,
            tags,
            cover_art,
        } => {
            tracing::info!("=== {} ({}) ===", path, format.to_uppercase());
            println!("{}:", tag_type);
            print_tag_map(tags, 2);
            if let Some(art) = cover_art {
                println!("  Cover Art    {} ({} bytes)", art.format, art.size_bytes);
            }
        }
        tags::FileReadResult::Wav {
            path,
            format,
            id3v2,
            riff_info,
            tag3_missing,
            cover_art,
        } => {
            tracing::info!("=== {} ({}) ===", path, format.to_uppercase());
            println!("ID3v2:");
            print_tag_map(id3v2, 2);
            println!();
            println!("RIFF INFO:");
            print_tag_map(riff_info, 2);
            if !tag3_missing.is_empty() {
                println!("  (not in RIFF INFO: {})", tag3_missing.join(", "));
            }
            if let Some(art) = cover_art {
                println!("  Cover Art    {} ({} bytes)", art.format, art.size_bytes);
            }
        }
        tags::FileReadResult::Error { path, error } => {
            tracing::error!("=== {} ===", path);
            tracing::error!("Error: {}", error);
        }
    }
}

fn print_tag_map(tags: &HashMap<String, Option<String>>, indent: usize) {
    let pad = " ".repeat(indent);
    // Print in canonical field order
    for &field in tags::ALL_FIELDS {
        if let Some(value) = tags.get(field) {
            let display = match value {
                Some(v) => v.as_str(),
                None => "(missing)",
            };
            println!("{pad}{:<14}{display}", display_field_name(field));
        }
    }
}

pub(crate) fn run_read_tags(args: ReadTagsArgs) -> Result<(), Box<dyn std::error::Error>> {
    let files = expand_paths(&args.paths, args.recursive);
    if files.is_empty() {
        return Err("No audio files found.".into());
    }

    let fields_ref = args.fields.as_deref();
    let mut had_errors = false;

    for (i, file) in files.iter().enumerate() {
        let result = tags::read_file_tags(file, fields_ref, args.cover_art);
        if matches!(&result, tags::FileReadResult::Error { .. }) {
            had_errors = true;
        }
        if args.json {
            println!("{}", serde_json::to_string(&result)?);
        } else {
            if i > 0 {
                println!();
            }
            print_tags_human(&result);
        }
    }

    if had_errors {
        std::process::exit(1);
    }

    Ok(())
}

fn build_tags_from_args(args: &WriteTagsArgs) -> HashMap<String, Option<String>> {
    let mut tags = HashMap::new();
    let fields: &[(&str, &Option<String>)] = &[
        ("artist", &args.artist),
        ("title", &args.title),
        ("album", &args.album),
        ("album_artist", &args.album_artist),
        ("genre", &args.genre),
        ("year", &args.year),
        ("track", &args.track),
        ("disc", &args.disc),
        ("comment", &args.comment),
        ("publisher", &args.publisher),
        ("bpm", &args.bpm),
        ("key", &args.key),
        ("composer", &args.composer),
        ("remixer", &args.remixer),
    ];
    for &(name, value) in fields {
        if let Some(v) = value {
            if v.is_empty() {
                // Empty string means delete the field
                tags.insert(name.to_string(), None);
            } else {
                tags.insert(name.to_string(), Some(v.clone()));
            }
        }
    }
    tags
}

fn parse_wav_targets(
    raw: &Option<Vec<String>>,
) -> Result<Vec<tags::WavTarget>, Box<dyn std::error::Error>> {
    match raw {
        Some(targets) => {
            let mut valid = Vec::new();
            let mut invalid = Vec::new();
            for t in targets {
                match t.as_str() {
                    "id3v2" => valid.push(tags::WavTarget::Id3v2),
                    "riff_info" => valid.push(tags::WavTarget::RiffInfo),
                    _ => invalid.push(t.as_str()),
                }
            }
            if !invalid.is_empty() {
                tracing::warn!("Unknown WAV target(s): {}", invalid.join(", "));
            }
            if valid.is_empty() {
                return Err("No valid WAV targets. Valid values: id3v2, riff_info"
                    .to_string()
                    .into());
            }
            Ok(valid)
        }
        None => Ok(vec![tags::WavTarget::Id3v2, tags::WavTarget::RiffInfo]),
    }
}

fn print_write_human(result: &tags::FileWriteResult) {
    match result {
        tags::FileWriteResult::Ok {
            path,
            fields_written,
            fields_deleted,
            wav_targets,
            ..
        } => {
            tracing::info!("=== {} ===", path);
            if !fields_written.is_empty() {
                println!("Written: {}", fields_written.join(", "));
            }
            if !fields_deleted.is_empty() {
                println!("Deleted: {}", fields_deleted.join(", "));
            }
            if fields_written.is_empty() && fields_deleted.is_empty() {
                println!("No changes.");
            }
            if let Some(targets) = wav_targets {
                println!("WAV targets: {}", targets.join(", "));
            }
        }
        tags::FileWriteResult::Error { path, error, .. } => {
            tracing::error!("=== {} ===", path);
            tracing::error!("Error: {}", error);
        }
    }
}

fn print_dry_run_human(result: &tags::FileDryRunResult) {
    match result {
        tags::FileDryRunResult::Preview {
            path,
            changes,
            wav_targets,
            ..
        } => {
            tracing::info!("=== {} (dry run) ===", path);
            if changes.is_empty() {
                println!("No changes.");
                return;
            }
            for &field in tags::ALL_FIELDS {
                if let Some(change) = changes.get(field) {
                    let old = change.old.as_deref().unwrap_or("(missing)");
                    let new = change.new.as_deref().unwrap_or("(delete)");
                    println!("  {:<14}{} -> {}", display_field_name(field), old, new);
                }
            }
            if let Some(targets) = wav_targets {
                println!("WAV targets: {}", targets.join(", "));
            }
        }
        tags::FileDryRunResult::Error { path, error, .. } => {
            tracing::error!("=== {} ===", path);
            tracing::error!("Error: {}", error);
        }
    }
}

pub(crate) fn run_write_tags(args: WriteTagsArgs) -> Result<(), Box<dyn std::error::Error>> {
    let tag_map = if args.json_input {
        let mut input = String::new();
        std::io::stdin().read_to_string(&mut input)?;
        let parsed: HashMap<String, Option<String>> = serde_json::from_str(&input)?;
        parsed
    } else {
        build_tags_from_args(&args)
    };

    if tag_map.is_empty() {
        return Err("No tags specified. Use --field flags or --json-input.".into());
    }

    let wav_targets = parse_wav_targets(&args.wav_targets)?;
    let entry = tags::WriteEntry {
        path: PathBuf::from(&args.path),
        tags: tag_map,
        wav_targets,
        comment_mode: tags::CommentMode::default(),
    };

    let had_errors;

    if args.dry_run {
        let result = tags::write_file_tags_dry_run(&entry);
        had_errors = matches!(&result, tags::FileDryRunResult::Error { .. });
        if args.json {
            println!("{}", serde_json::to_string(&result)?);
        } else {
            print_dry_run_human(&result);
        }
    } else {
        let result = tags::write_file_tags(&entry);
        had_errors = matches!(&result, tags::FileWriteResult::Error { .. });
        if args.json {
            println!("{}", serde_json::to_string(&result)?);
        } else {
            print_write_human(&result);
        }
    }

    if had_errors {
        std::process::exit(1);
    }

    Ok(())
}

pub(crate) fn run_extract_art(args: ExtractArtArgs) -> Result<(), Box<dyn std::error::Error>> {
    let path = Path::new(&args.path);
    let output = args.output.as_deref().map(Path::new);

    match tags::extract_cover_art(path, output, &args.picture_type) {
        Ok(result) => {
            if args.json {
                println!("{}", serde_json::to_string(&result)?);
            } else {
                println!(
                    "Extracted {} ({}, {} bytes) -> {}",
                    result.picture_type, result.image_format, result.size_bytes, result.output_path
                );
            }
        }
        Err(e) => {
            tracing::error!("Error: {}", e);
            std::process::exit(1);
        }
    }

    Ok(())
}

pub(crate) fn run_embed_art(args: EmbedArtArgs) -> Result<(), Box<dyn std::error::Error>> {
    let image_path = Path::new(&args.image);
    let mut had_errors = false;

    for target in &args.targets {
        let target_path = Path::new(target);
        let result = tags::embed_cover_art(image_path, target_path, &args.picture_type);

        if matches!(&result, tags::FileEmbedResult::Error { .. }) {
            had_errors = true;
        }

        if args.json {
            println!("{}", serde_json::to_string(&result)?);
        } else {
            match &result {
                tags::FileEmbedResult::Ok { path, .. } => {
                    println!("Embedded cover art into {}", path);
                }
                tags::FileEmbedResult::Error { path, error, .. } => {
                    tracing::error!("Error ({}): {}", path, error);
                }
            }
        }
    }

    if had_errors {
        std::process::exit(1);
    }

    Ok(())
}
