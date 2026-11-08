//! Is the content half of the mod there? The native half can't add the blocks, models
//! or textures: without the content mod (`horizon_create`, in `horizon-mods/` and turned
//! on in Options > Mods, read at start) nothing shows up and the game only logs
//! "standalone model ... not loaded". This finds out which step is missing, so the
//! native mod can say it in chat.

use std::path::Path;

pub const CONTENT_ID: &str = "horizon_create";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentStatus {
    /// No `.hmod` / folder of the content mod in `horizon-mods/`.
    NotInstalled,
    /// Installed, but its switch (`mod.horizon_create:true` in `horizon-mods.txt`) is off.
    TurnedOff,
    TurnedOn,
}

/// Look at the instance folder (the one with `options.txt`).
pub fn content_status(instance: &Path) -> ContentStatus {
    let mods = instance.join("horizon-mods");
    let installed = std::fs::read_dir(&mods).is_ok_and(|entries| {
        entries.flatten().any(|e| {
            let path = e.path();
            let name = e.file_name().to_string_lossy().to_lowercase();
            if path.is_dir() {
                std::fs::read_to_string(path.join("horizon-mod.toml"))
                    .is_ok_and(|t| declares_id(&t, CONTENT_ID))
            } else {
                name.ends_with(".hmod") && name.starts_with(CONTENT_ID)
            }
        })
    });
    if !installed {
        return ContentStatus::NotInstalled;
    }
    let on = std::fs::read_to_string(instance.join("horizon-mods.txt")).is_ok_and(|t| {
        t.lines()
            .any(|l| l.trim() == format!("mod.{CONTENT_ID}:true"))
    });
    if on {
        ContentStatus::TurnedOn
    } else {
        ContentStatus::TurnedOff
    }
}

fn declares_id(manifest: &str, id: &str) -> bool {
    manifest.lines().any(|l| {
        let mut kv = l.splitn(2, '=');
        kv.next().map(str::trim) == Some("id")
            && kv.next().map(|v| v.trim().trim_matches('"')) == Some(id)
    })
}

/// The chat line for a player, or `None` when everything is in place.
pub fn status_message(status: ContentStatus, instance: &Path) -> Option<String> {
    let folder = instance.join("horizon-mods");
    match status {
        ContentStatus::TurnedOn => None,
        ContentStatus::NotInstalled => Some(format!(
            "[Horizon Create] The content mod is missing: put horizon_create-<version>.hmod in {}, \
             turn it on in Options > Mods and restart the game.",
            folder.display()
        )),
        ContentStatus::TurnedOff => Some(
            "[Horizon Create] The content mod is installed but turned off: turn on \"Horizon Create\" \
             in Options > Mods and restart the game."
                .to_string(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("hcglue-setup-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("horizon-mods")).unwrap();
        dir
    }

    #[test]
    fn missing_off_and_on() {
        let dir = instance("file");
        assert_eq!(content_status(&dir), ContentStatus::NotInstalled);
        assert!(status_message(ContentStatus::NotInstalled, &dir)
            .unwrap()
            .contains("horizon-mods"));

        std::fs::write(dir.join("horizon-mods/horizon_create-0.2.3.hmod"), b"zip").unwrap();
        assert_eq!(content_status(&dir), ContentStatus::TurnedOff);

        std::fs::write(
            dir.join("horizon-mods.txt"),
            "mod.other:true\nmod.horizon_create:false\n",
        )
        .unwrap();
        assert_eq!(content_status(&dir), ContentStatus::TurnedOff);

        std::fs::write(dir.join("horizon-mods.txt"), "mod.horizon_create:true\n").unwrap();
        assert_eq!(content_status(&dir), ContentStatus::TurnedOn);
        assert_eq!(status_message(ContentStatus::TurnedOn, &dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mod_folder_counts_by_its_manifest() {
        let dir = instance("folder");
        std::fs::create_dir_all(dir.join("horizon-mods/whatever")).unwrap();
        std::fs::write(
            dir.join("horizon-mods/whatever/horizon-mod.toml"),
            "id = \"horizon_create\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("horizon-mods.txt"), "mod.horizon_create:true\n").unwrap();
        assert_eq!(content_status(&dir), ContentStatus::TurnedOn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_folder_is_not_installed() {
        let dir = std::env::temp_dir().join("hcglue-setup-does-not-exist");
        assert_eq!(content_status(&dir), ContentStatus::NotInstalled);
    }
}
