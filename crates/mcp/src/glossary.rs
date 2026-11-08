//! Names the user saved so speech recognition spells them their way, such as Claude Code or Nůžky. They stay in
//! Nuzky's data folder and reach only the local Whisper model, as a hint; nothing changes a transcript already made.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use unicode_normalization::UnicodeNormalization;

/// As many names as fit in the speech model's hint with room to spare, so none is left out.
pub const MAX_NAMES: usize = 40;
pub const MAX_NAME_CHARS: usize = 40;

fn path() -> Result<PathBuf> {
    Ok(nuzky_engine::data_dir()
        .context("NAMES_UNWRITABLE: this computer has no folder for Nuzky's data")?
        .join("glossary.json"))
}

/// A name as it is saved: one line with single spaces, its accents composed as a keyboard types them.
pub fn check_name(text: &str) -> Result<String> {
    let name: String = text.split_whitespace().collect::<Vec<_>>().join(" ").nfc().collect();
    ensure!(
        (1..=MAX_NAME_CHARS).contains(&name.chars().count())
            && name.chars().any(char::is_alphanumeric)
            && !name.chars().any(char::is_control),
        "NAME_INVALID: a name is one line of 1 to {MAX_NAME_CHARS} characters with a letter or digit"
    );
    Ok(name)
}

/// Two spellings that differ only in case are one name; ones with different accents, Nůžky and Nuzky, are two.
fn same(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// The saved names, oldest first.
pub fn list() -> Result<Vec<String>> {
    read(&path()?)
}

/// Saves `name`, or its new spelling when only its case differs, as the newest name.
pub fn add(name: &str) -> Result<Vec<String>> {
    add_at(&path()?, name)
}

/// Spells the saved name `from` as `to`, in its place.
pub fn rename(from: &str, to: &str) -> Result<Vec<String>> {
    rename_at(&path()?, from, to)
}

pub fn remove(name: &str) -> Result<Vec<String>> {
    remove_at(&path()?, name)
}

fn add_at(path: &Path, name: &str) -> Result<Vec<String>> {
    change(path, |names| {
        let name = check_name(name)?;
        let had = names.len();
        names.retain(|n| !same(n, &name));
        ensure!(
            names.len() < had || names.len() < MAX_NAMES,
            "NAMES_FULL: you have saved {MAX_NAMES} names, the most Nuzky keeps; delete one you no longer say first"
        );
        names.push(name);
        Ok(())
    })
}

fn rename_at(path: &Path, from: &str, to: &str) -> Result<Vec<String>> {
    change(path, |names| {
        let to = check_name(to)?;
        let at = names
            .iter()
            .position(|n| n == from)
            .with_context(|| format!("NAME_UNKNOWN: {from:?} is no longer saved"))?;
        ensure!(
            !names.iter().enumerate().any(|(i, n)| i != at && same(n, &to)),
            "NAME_EXISTS: {to:?} is already saved"
        );
        names[at] = to;
        Ok(())
    })
}

fn remove_at(path: &Path, name: &str) -> Result<Vec<String>> {
    change(path, |names| {
        names.retain(|n| n != name);
        Ok(())
    })
}

fn read(path: &Path) -> Result<Vec<String>> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(error) => return Err(error).context("NAMES_UNREADABLE: your saved names could not be read"),
    };
    let names: Vec<String> =
        serde_json::from_slice(&bytes).context("NAMES_UNREADABLE: the file of your saved names is damaged")?;
    // A file edited by hand still gives the speech model only names it can read.
    Ok(names.iter().filter_map(|n| check_name(n).ok()).take(MAX_NAMES).collect())
}

fn change(path: &Path, edit: impl FnOnce(&mut Vec<String>) -> Result<()>) -> Result<Vec<String>> {
    let mut names = read(path)?;
    edit(&mut names)?;
    std::fs::create_dir_all(path.parent().context("NAMES_UNWRITABLE: no folder for your saved names")?)
        .context("NAMES_UNWRITABLE: your names could not be saved")?;
    nuzky_session::write_json_atomic(path, &names).context("NAMES_UNWRITABLE: your names could not be saved")?;
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (PathBuf, impl Drop) {
        struct Gone(PathBuf);
        impl Drop for Gone {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let dir = std::env::temp_dir().join(format!("nuzky-glossary-{}", nuzky_engine::edit::new_id()));
        (dir.join("glossary.json"), Gone(dir))
    }

    #[test]
    fn names_keep_accents_and_words_and_refuse_what_is_not_a_name() {
        assert_eq!(check_name("  Claude \t Code ").unwrap(), "Claude Code");
        // A decomposed "ů", as some apps paste it, is the name typed on a keyboard.
        assert_eq!(check_name("Nu\u{30a}z\u{30c}ky").unwrap(), "Nůžky");
        for bad in ["", "   ", "...", "a\u{7}b", &"x".repeat(MAX_NAME_CHARS + 1)] {
            assert!(check_name(bad).unwrap_err().to_string().starts_with("NAME_INVALID"), "{bad:?}");
        }
        assert!(same("codex", "Codex") && !same("Nůžky", "Nuzky"));
    }

    #[test]
    fn saved_names_last_and_one_spelling_replaces_another() {
        let (path, _gone) = store();
        assert!(read(&path).unwrap().is_empty());
        add_at(&path, "Nůžky").unwrap();
        add_at(&path, "Claude Code").unwrap();
        add_at(&path, "Nuzky").unwrap();
        // The same name again, cased differently, moves to the newest place with its new spelling.
        add_at(&path, "nůžky").unwrap();
        assert_eq!(read(&path).unwrap(), ["Claude Code", "Nuzky", "nůžky"]);
        remove_at(&path, "Nuzky").unwrap();
        assert_eq!(read(&path).unwrap(), ["Claude Code", "nůžky"]);
        assert_eq!(rename_at(&path, "nůžky", "Nůžky").unwrap(), ["Claude Code", "Nůžky"]);
        assert!(rename_at(&path, "Nůžky", "claude code").unwrap_err().to_string().starts_with("NAME_EXISTS"));
        assert!(rename_at(&path, "Codex", "Codex CLI").unwrap_err().to_string().starts_with("NAME_UNKNOWN"));
        for i in 2..MAX_NAMES {
            add_at(&path, &format!("Name {i}")).unwrap();
        }
        assert!(add_at(&path, "One too many").unwrap_err().to_string().starts_with("NAMES_FULL"));
        // Saving a name that is there already is never refused.
        assert_eq!(add_at(&path, "CLAUDE CODE").unwrap().last().unwrap(), "CLAUDE CODE");
    }

    #[test]
    fn a_damaged_file_is_reported_and_never_overwritten() {
        let (path, _gone) = store();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{not json").unwrap();
        assert!(add_at(&path, "Codex").unwrap_err().to_string().starts_with("NAMES_UNREADABLE"));
        assert_eq!(std::fs::read(&path).unwrap(), b"{not json");
    }
}
