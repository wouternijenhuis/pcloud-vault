use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use vault_core::{EntryKind, Vault};
use zeroize::Zeroizing;

fn main() {
    if let Err(error) = run() {
        eprintln!("Recovery failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "hydrate") {
        return hydrate(&args);
    }
    if args.len() < 2 || !matches!(args[0].as_str(), "verify" | "export") {
        return Err("Usage: vault-recovery verify <vault-directory> [--recovery] [--password-stdin]\n       vault-recovery export <vault-directory> <new-output-directory> [--recovery] [--password-stdin]\n       vault-recovery hydrate <downloaded-cloud-directory> <new-vault-directory>\nSecrets are requested interactively, never on the command line. --password-stdin reads one line from an explicit pipe.".into());
    }
    let root = Path::new(&args[1]);
    let recovery = args.iter().any(|arg| arg == "--recovery");
    let secret = if args.iter().any(|arg| arg == "--password-stdin") {
        let mut bytes = Zeroizing::new(Vec::new());
        std::io::stdin().take(4097).read_to_end(&mut bytes)?;
        if bytes.len() > 4096 {
            return Err("Secret input is too long".into());
        }
        Zeroizing::new(
            String::from_utf8(bytes.to_vec())?
                .trim_end_matches(['\r', '\n'])
                .to_string(),
        )
    } else {
        Zeroizing::new(rpassword::prompt_password(if recovery {
            "Recovery key: "
        } else {
            "Vault password: "
        })?)
    };
    let vault = if recovery {
        Vault::open_with_recovery(root, &secret)
    } else {
        Vault::open(root, &secret)
    }
    .map_err(|error| format!("Could not open the vault at {}: {error}", root.display()))?;
    let entries = vault.all_entries();
    let destination = if args[0] == "export" {
        let path = PathBuf::from(
            args.get(2)
                .filter(|arg| arg.as_str() != "--recovery")
                .ok_or("Export needs an output directory")?,
        );
        if path.exists() {
            return Err(
                "Output directory already exists; choose a new directory to prevent overwrites"
                    .into(),
            );
        }
        std::fs::create_dir_all(&path).map_err(|error| {
            format!(
                "Could not create the output folder {}: {error}",
                path.display()
            )
        })?;
        Some(path)
    } else {
        None
    };
    let mut verified = 0u64;
    let mut buffer = Zeroizing::new(vec![0u8; 4 * 1024 * 1024]);
    for entry in &entries {
        if entry.trashed {
            continue;
        }
        let is_folder = entry.kind == EntryKind::Folder;
        if let Some(output) = &destination {
            let mut components = vec![entry.name.clone()];
            let mut parent = entry.parent_id.clone();
            let mut visited = HashSet::new();
            while let Some(id) = parent {
                if !visited.insert(id.clone()) {
                    return Err("Invalid directory cycle".into());
                }
                let ancestor = entries
                    .iter()
                    .find(|item| item.id == id)
                    .ok_or("Missing parent directory")?;
                components.push(ancestor.name.clone());
                parent = ancestor.parent_id.clone();
            }
            let mut path = output.clone();
            for name in components.iter().rev() {
                if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', ':'])
                {
                    return Err("Unsafe entry name in vault".into());
                }
                path.push(name);
            }
            if is_folder {
                std::fs::create_dir_all(path)?;
            } else {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                vault
                    .export_file(&entry.id, &path)
                    .map_err(|error| format!("Could not export {}: {error}", path.display()))?;
                verified += 1;
            }
        } else if !is_folder {
            let mut offset = 0;
            loop {
                let count = vault.read(&entry.id, offset, &mut buffer)?;
                if count == 0 {
                    break;
                }
                offset += count as u64;
            }
            if offset != entry.size {
                return Err("File size did not match its authenticated manifest".into());
            }
            verified += 1;
        }
    }
    println!(
        "{} {verified} files successfully.",
        if destination.is_some() {
            "Exported"
        } else {
            "Verified"
        }
    );
    Ok(())
}

fn hydrate(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if args.len() != 3 {
        return Err(
            "Usage: vault-recovery hydrate <downloaded-cloud-directory> <new-vault-directory>"
                .into(),
        );
    }
    let source = Path::new(&args[1]);
    let destination = Path::new(&args[2]);
    if destination.exists() {
        return Err("Choose a new destination directory".into());
    }
    let header = Vault::read_header(source)?;
    std::fs::create_dir_all(destination.join("objects"))?;
    let mut output = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(destination.join("vault.json"))?;
    use std::io::Write;
    output.write_all(&serde_json::to_vec_pretty(&header)?)?;
    output.sync_all()?;
    let mut count = 0;
    for object in std::fs::read_dir(source.join("objects"))? {
        let object = object?;
        if !object.file_type()?.is_file() {
            continue;
        }
        let name = object.file_name().to_string_lossy().into_owned();
        let Some(stem) = name.strip_suffix(".obj") else {
            continue;
        };
        let id = if let Some((kind, id)) = stem.split_once('-') {
            if !matches!(kind, "c" | "m" | "h") {
                return Err("Unknown object kind in cloud download".into());
            }
            id
        } else {
            stem
        };
        if id.len() != 32
            || !id
                .bytes()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        {
            return Err("Unsafe object filename in cloud download".into());
        }
        let mut source_file = std::fs::File::open(object.path())?;
        let mut target = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(destination.join("objects").join(format!("{id}.obj")))?;
        std::io::copy(&mut source_file, &mut target)?;
        target.sync_all()?;
        count += 1;
    }
    println!("Prepared {count} encrypted objects. Run verify before exporting plaintext.");
    Ok(())
}
