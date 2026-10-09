//! Resolve service identities before spawning. Unix credentials are changed
//! only in the child, before R is executed; the gateway keeps its own identity.
use crate::config::App;
use anyhow::{Result, bail};
use std::path::{Path, PathBuf};

#[cfg(unix)]
pub struct Identity {
    user: nix::unistd::User,
    groups: Vec<nix::unistd::Gid>,
}
#[cfg(not(unix))]
pub struct Identity;

pub fn resolve(app: &App) -> Result<Option<Identity>> {
    for name in &app.run_as {
        if name == ":PROCESS_USER:" {
            return Ok(None);
        }
        #[cfg(unix)]
        {
            use nix::unistd::{Uid, User, geteuid};
            let user = if name == ":HOME_USER:" {
                use std::os::unix::fs::MetadataExt;
                User::from_uid(Uid::from_raw(std::fs::metadata(&app.path)?.uid()))?
            } else {
                User::from_name(name)?
            };
            if let Some(user) = user {
                if user.uid == geteuid() {
                    return Ok(None);
                }
                anyhow::ensure!(
                    geteuid().is_root(),
                    "run_as {} requires a privileged service or running the service as that account",
                    user.name
                );
                let groups = account_groups(&user)?;
                return Ok(Some(Identity { user, groups }));
            }
        }
        #[cfg(not(unix))]
        {
            // Service Control Manager establishes Windows identities; changing
            // accounts requires an account token, never a plaintext password.
            if std::env::var("USERNAME").is_ok_and(|user| user.eq_ignore_ascii_case(name)) {
                return Ok(None);
            }
        }
    }
    bail!(
        "None of the configured run_as accounts can be used: {:?}",
        app.run_as
    )
}

pub fn home(name: &str) -> Result<PathBuf> {
    #[cfg(unix)]
    {
        Ok(nix::unistd::User::from_name(name)?
            .ok_or_else(|| anyhow::anyhow!("Unknown OS account"))?
            .dir)
    }
    #[cfg(not(unix))]
    {
        anyhow::ensure!(
            std::env::var("USERNAME").is_ok_and(|user| user.eq_ignore_ascii_case(name)),
            "Windows user_dirs supports the service account"
        );
        Ok(std::env::var_os("USERPROFILE")
            .ok_or_else(|| anyhow::anyhow!("Missing user profile"))?
            .into())
    }
}

pub fn home_if_member(name: &str, required: &[String]) -> Result<PathBuf> {
    if required.is_empty() {
        return home(name);
    }
    #[cfg(unix)]
    {
        use nix::unistd::{Group, User};
        let user = User::from_name(name)?.ok_or_else(|| anyhow::anyhow!("Unknown OS account"))?;
        let actual = account_groups(&user)?;
        for group in required {
            if let Some(group) = Group::from_name(group)?
                && actual.contains(&group.gid)
            {
                return Ok(user.dir);
            }
        }
        bail!("OS account is not in an allowed deployment group")
    }
    #[cfg(not(unix))]
    bail!("members_of requires Unix OS group lookup")
}

#[cfg(all(unix, not(target_vendor = "apple")))]
fn account_groups(user: &nix::unistd::User) -> Result<Vec<nix::unistd::Gid>> {
    Ok(nix::unistd::getgrouplist(
        &std::ffi::CString::new(user.name.as_str())?,
        user.gid,
    )?)
}

// Darwin's getgrouplist ABI uses signed integers, so nix does not expose it.
// Perform the directory lookup in the parent, never in the post-fork hook.
#[cfg(target_vendor = "apple")]
#[allow(unsafe_code)]
fn account_groups(user: &nix::unistd::User) -> Result<Vec<nix::unistd::Gid>> {
    let name = std::ffi::CString::new(user.name.as_str())?;
    let mut groups = vec![0_i32; 16];
    loop {
        let mut count = groups.len() as nix::libc::c_int;
        // SAFETY: name is NUL-terminated; groups contains count writable c_ints,
        // and count points to a live c_int. No pointers escape the call.
        let result = unsafe {
            nix::libc::getgrouplist(
                name.as_ptr(),
                user.gid.as_raw() as nix::libc::c_int,
                groups.as_mut_ptr(),
                &mut count,
            )
        };
        let count = usize::try_from(count)?;
        anyhow::ensure!(count <= 65536, "OS account has too many groups");
        if result >= 0 {
            anyhow::ensure!(count <= groups.len(), "Invalid OS group lookup result");
            groups.truncate(count);
            return Ok(groups
                .into_iter()
                .map(|g| nix::unistd::Gid::from_raw(g as u32))
                .collect());
        }
        anyhow::ensure!(count > groups.len(), "Cannot resolve OS account groups");
        groups.resize(count, 0);
    }
}

#[cfg(all(test, unix))]
mod tests {
    #[test]
    fn deployment_groups_include_primary_group_and_deny_unknown_membership() {
        use nix::unistd::{Group, User, geteuid};
        let user = User::from_uid(geteuid()).unwrap().unwrap();
        let group = Group::from_gid(user.gid).unwrap().unwrap();
        assert_eq!(
            super::home_if_member(&user.name, &[group.name]).unwrap(),
            user.dir
        );
        assert!(
            super::home_if_member(&user.name, &["shinyharbor-nonexistent-test-group".into()])
                .is_err()
        );
    }
}

impl Identity {
    /// Give only this worker account access to a newly created artifact.
    pub fn own(&self, path: &Path) -> Result<()> {
        #[cfg(unix)]
        nix::unistd::chown(path, Some(self.user.uid), Some(self.user.gid))?;
        #[cfg(not(unix))]
        let _ = path;
        Ok(())
    }
    pub fn bookmarks(&self, state: &Path) -> Result<PathBuf> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            // Keep the gateway's keys/admin token private. Per-user persistent
            // data gets its own sibling tree instead of opening state_dir.
            let root = state.with_extension("users");
            if !root.exists() {
                std::fs::create_dir(&root)?;
                std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o711))?;
            }
            anyhow::ensure!(
                !std::fs::symlink_metadata(&root)?.file_type().is_symlink(),
                "User data root must not be a symlink"
            );
            let metadata = std::fs::metadata(&root)?;
            anyhow::ensure!(
                metadata.uid() == nix::unistd::geteuid().as_raw() && metadata.mode() & 0o022 == 0,
                "User data root must be owned by the service account and not writable by others"
            );
            let directory = root.join(self.user.uid.to_string());
            crate::security::private_dir(&directory)?;
            self.own(&directory)?;
            Ok(directory)
        }
        #[cfg(not(unix))]
        {
            Ok(state.join("bookmarks"))
        }
    }
    pub fn prepare_bookmarks(&self, root: &Path) -> Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if !root.exists() {
                std::fs::create_dir_all(root)?;
                std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o711))?;
            }
            anyhow::ensure!(
                root.is_dir() && !std::fs::symlink_metadata(root)?.file_type().is_symlink(),
                "Invalid bookmark root"
            );
            let account = root.join(&self.user.name);
            crate::security::private_dir(&account)?;
            self.own(&account)?;
        }
        #[cfg(not(unix))]
        let _ = root;
        Ok(())
    }
    #[cfg(unix)]
    #[allow(unsafe_code)]
    pub fn apply(&self, command: &mut tokio::process::Command) {
        let uid = self.user.uid;
        let gid = self.user.gid;
        let groups = self.groups.clone();
        #[cfg(target_vendor = "apple")]
        let groups: Vec<nix::libc::gid_t> = groups.iter().map(|g| g.as_raw()).collect();
        command
            .env("HOME", &self.user.dir)
            .env("USER", &self.user.name)
            .env("LOGNAME", &self.user.name);
        // SAFETY: every allocation and NSS/group lookup happens before fork.
        // The child hook performs only setgroups/setgid/setuid system calls;
        // no locks, logging, allocation, or runtime operations occur here.
        unsafe {
            command.pre_exec(move || {
                #[cfg(not(target_vendor = "apple"))]
                nix::unistd::setgroups(&groups).map_err(std::io::Error::from)?;
                #[cfg(target_vendor = "apple")]
                {
                    // SAFETY: group storage was allocated before fork, is alive
                    // for the call and contains at most 65536 valid gid_t values.
                    // setgroups is a syscall; it performs no directory lookup.
                    if nix::libc::setgroups(groups.len() as nix::libc::c_int, groups.as_ptr()) != 0
                    {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                nix::unistd::setgid(gid).map_err(std::io::Error::from)?;
                nix::unistd::setuid(uid).map_err(std::io::Error::from)?;
                Ok(())
            });
        }
    }
    #[cfg(not(unix))]
    pub fn apply(&self, _command: &mut tokio::process::Command) {}
}
