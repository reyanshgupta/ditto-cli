//! Codex executes plugins only from trusted roots. Its loader resolves links,
//! so sharing the cache through a link outside CODEX_HOME prevents startup.

use std::{fs, io, path::Path};

use anyhow::{Context, Result, bail};

use crate::profile::{Profile, secure_directory, write_private_bytes};

/// Copies only missing plugin code. Runtime state beside the cache belongs to
/// the profile, while the shared source and existing local files stay intact.
pub fn sync(source: &Profile, target: &Profile) -> Result<bool> {
    if !target.managed || source.codex_home == target.codex_home {
        return Ok(false);
    }
    let from = source.codex_home.join("plugins");
    let into = target.codex_home.join("plugins");
    match fs::symlink_metadata(&into) {
        Ok(metadata) if metadata.is_symlink() => {
            let expected = fs::canonicalize(&from)
                .with_context(|| format!("could not resolve {}", from.display()))?;
            if fs::canonicalize(&into)? != expected {
                bail!(
                    "{} is a custom plugin link; leave it intact and install plugins inside the profile before launching Codex",
                    into.display()
                );
            }
            migrate(&from, &into)?;
            Ok(true)
        }
        Ok(metadata) if !metadata.is_dir() => {
            bail!("{} is not a plugin directory", into.display());
        }
        Ok(_) => copy_missing(&from.join("cache"), &into.join("cache")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            create_directory(&into)?;
            copy_missing(&from.join("cache"), &into.join("cache"))
        }
        Err(error) => Err(error).with_context(|| format!("could not inspect {}", into.display())),
    }
}

fn migrate(from: &Path, into: &Path) -> Result<()> {
    let stage = into.with_file_name(format!("plugins.ditto-local-{}", std::process::id()));
    // Exclusive creation avoids following an old staging link or overwriting
    // files left by an interrupted migration.
    create_directory(&stage)?;
    let result = (|| {
        copy_missing(&from.join("cache"), &stage.join("cache"))?;
        let mut backup = into.with_file_name("plugins.before-ditto");
        for n in 1.. {
            match fs::symlink_metadata(&backup) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => break,
                Err(error) => return Err(error.into()),
                Ok(_) => backup = into.with_file_name(format!("plugins.before-ditto.{n}")),
            }
        }
        fs::rename(into, &backup)
            .with_context(|| format!("could not preserve {}", into.display()))?;
        if let Err(error) = fs::rename(&stage, into) {
            fs::rename(&backup, into).with_context(|| {
                format!(
                    "could not restore {} from {}",
                    into.display(),
                    backup.display()
                )
            })?;
            return Err(error).with_context(|| format!("could not install {}", into.display()));
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&stage);
    }
    result
}

fn create_directory(path: &Path) -> Result<()> {
    fs::create_dir(path).with_context(|| format!("could not create {}", path.display()))?;
    secure_directory(path)
}

fn copy_missing(from: &Path, into: &Path) -> Result<bool> {
    let source = match fs::symlink_metadata(from) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error).with_context(|| format!("could not inspect {}", from.display()));
        }
    };
    // Following arbitrary cache links could import files outside the plugin
    // installation, while preserving them would recreate the trust failure.
    if source.is_symlink() || !(source.is_dir() || source.is_file()) {
        bail!(
            "{} is not a regular plugin file or directory; install this plugin directly in the profile",
            from.display()
        );
    }
    match fs::symlink_metadata(into) {
        Ok(target) if target.is_symlink() => {
            bail!(
                "{} is a linked plugin path; install this plugin directly in the profile",
                into.display()
            );
        }
        Ok(target) if source.is_file() || !target.is_dir() => return Ok(false),
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if source.is_file() {
                let contents =
                    fs::read(from).with_context(|| format!("could not read {}", from.display()))?;
                write_private_bytes(into, &contents)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    // Plugin helpers must remain executable, without granting
                    // other accounts access to the profile's copy.
                    let mode = 0o600 | (source.permissions().mode() & 0o100);
                    fs::set_permissions(into, fs::Permissions::from_mode(mode))?;
                }
                return Ok(true);
            }
            create_directory(into)?;
        }
        Err(error) => {
            return Err(error).with_context(|| format!("could not inspect {}", into.display()));
        }
    }
    let mut changed = false;
    for entry in fs::read_dir(from).with_context(|| format!("could not read {}", from.display()))? {
        let entry =
            entry.with_context(|| format!("could not read an entry in {}", from.display()))?;
        changed |= copy_missing(&entry.path(), &into.join(entry.file_name()))?;
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{DEFAULT_PROFILE, Store};
    use tempfile::tempdir;

    #[test]
    fn copies_plugin_code_but_keeps_runtime_state_and_credentials_private() {
        let temp = tempdir().unwrap();
        let store = Store::new(temp.path().join("ditto"), temp.path().join("home"));
        let source = store.load_profile(DEFAULT_PROFILE).unwrap();
        let target = store.create_profile("work").unwrap();
        write_private_bytes(
            &source
                .codex_home
                .join("plugins/cache/browser/1/service.mjs"),
            b"code",
        )
        .unwrap();
        write_private_bytes(
            &source.codex_home.join("plugins/.plugin-appserver/state"),
            b"state",
        )
        .unwrap();
        write_private_bytes(&source.codex_home.join("auth.json"), b"auth").unwrap();
        assert!(sync(&source, &target).unwrap());
        let service = target
            .codex_home
            .join("plugins/cache/browser/1/service.mjs");
        assert_eq!(fs::read(&service).unwrap(), b"code");
        assert!(
            fs::canonicalize(service)
                .unwrap()
                .starts_with(fs::canonicalize(&target.codex_home).unwrap())
        );
        assert!(!target.codex_home.join("plugins/.plugin-appserver").exists());
        assert!(!target.codex_home.join("auth.json").exists());
        assert!(!sync(&source, &target).unwrap());
        write_private_bytes(
            &target
                .codex_home
                .join("plugins/cache/browser/1/service.mjs"),
            b"local",
        )
        .unwrap();
        write_private_bytes(
            &source
                .codex_home
                .join("plugins/cache/browser/2/service.mjs"),
            b"update",
        )
        .unwrap();
        assert!(sync(&source, &target).unwrap());
        assert_eq!(
            fs::read(
                target
                    .codex_home
                    .join("plugins/cache/browser/1/service.mjs")
            )
            .unwrap(),
            b"local"
        );
        assert_eq!(
            fs::read(
                target
                    .codex_home
                    .join("plugins/cache/browser/2/service.mjs")
            )
            .unwrap(),
            b"update"
        );
    }

    #[cfg(unix)]
    #[test]
    fn migrates_the_shared_link_without_changing_the_source_or_earlier_backups() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let temp = tempdir().unwrap();
        let store = Store::new(temp.path().join("ditto"), temp.path().join("home"));
        let source = store.load_profile(DEFAULT_PROFILE).unwrap();
        let target = store.create_profile("work").unwrap();
        let helper = source.codex_home.join("plugins/cache/browser/helper");
        write_private_bytes(&helper, b"helper").unwrap();
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o755)).unwrap();
        symlink(
            source.codex_home.join("plugins"),
            target.codex_home.join("plugins"),
        )
        .unwrap();
        write_private_bytes(&target.codex_home.join("plugins.before-ditto"), b"backup").unwrap();
        assert!(sync(&source, &target).unwrap());
        assert!(
            !fs::symlink_metadata(target.codex_home.join("plugins"))
                .unwrap()
                .is_symlink()
        );
        assert_eq!(fs::read(&helper).unwrap(), b"helper");
        assert_eq!(
            fs::read(target.codex_home.join("plugins.before-ditto")).unwrap(),
            b"backup"
        );
        assert_eq!(
            fs::read_link(target.codex_home.join("plugins.before-ditto.1")).unwrap(),
            source.codex_home.join("plugins")
        );
        let local = target.codex_home.join("plugins/cache/browser/helper");
        assert_eq!(
            fs::metadata(local).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_copy_leaves_the_old_link_and_external_files_untouched() {
        use std::os::unix::fs::symlink;
        let temp = tempdir().unwrap();
        let store = Store::new(temp.path().join("ditto"), temp.path().join("home"));
        let source = store.load_profile(DEFAULT_PROFILE).unwrap();
        let target = store.create_profile("work").unwrap();
        write_private_bytes(&source.codex_home.join("plugins/cache/service"), b"code").unwrap();
        let secret = temp.path().join("private");
        write_private_bytes(&secret, b"private").unwrap();
        symlink(&secret, source.codex_home.join("plugins/cache/external")).unwrap();
        symlink(
            source.codex_home.join("plugins"),
            target.codex_home.join("plugins"),
        )
        .unwrap();
        assert!(sync(&source, &target).is_err());
        assert!(
            fs::symlink_metadata(target.codex_home.join("plugins"))
                .unwrap()
                .is_symlink()
        );
        assert!(!target.codex_home.join("plugins.before-ditto").exists());
        assert_eq!(fs::read(secret).unwrap(), b"private");
    }

    #[cfg(unix)]
    #[test]
    fn keeps_custom_plugin_links_and_rejects_linked_cache_destinations() {
        use std::os::unix::fs::symlink;
        let temp = tempdir().unwrap();
        let store = Store::new(temp.path().join("ditto"), temp.path().join("home"));
        let source = store.load_profile(DEFAULT_PROFILE).unwrap();
        let target = store.create_profile("work").unwrap();
        write_private_bytes(&source.codex_home.join("plugins/cache/service"), b"source").unwrap();
        let custom = temp.path().join("custom");
        write_private_bytes(&custom.join("service"), b"custom").unwrap();
        let plugins = target.codex_home.join("plugins");
        symlink(&custom, &plugins).unwrap();
        assert!(sync(&source, &target).is_err());
        assert_eq!(fs::read_link(&plugins).unwrap(), custom);
        fs::remove_file(&plugins).unwrap();
        create_directory(&plugins).unwrap();
        symlink(&custom, plugins.join("cache")).unwrap();
        assert!(sync(&source, &target).is_err());
        assert_eq!(fs::read(custom.join("service")).unwrap(), b"custom");
    }

    #[test]
    fn syncing_the_default_profile_does_not_create_plugin_files() {
        let temp = tempdir().unwrap();
        let store = Store::new(temp.path().join("ditto"), temp.path().join("home"));
        let source = store.load_profile(DEFAULT_PROFILE).unwrap();
        assert!(!sync(&source, &source).unwrap());
        assert!(!source.codex_home.join("plugins").exists());
    }
}
